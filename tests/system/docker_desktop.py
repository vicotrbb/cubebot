"""The console desktop view (ADRs 0039 and 0085) on real agents through Docker.

Same host needs as docker_sandbox.py, plus Node with Playwright's Chromium
(CUBEBOT_PLAYWRIGHT_MODULE) for the console check. The Desk agent runs the guest desktop
with OpenAI's computer tool; the Idle agent is never woken. An administrator, a viewer
with `browser.view` on Desk, a controller with `browser.view` and `browser.control` on
Desk, and an outsider with no grants each open GET /api/agents/{id}/desktop.

Human input is proved where it lands: the controller types a URL into the guest's
Chromium and clicks a link on the page it opens, and both requests must reach a server
this script runs. Agent refusals and re-observation are read from the ledger and the
arbiter's log, never from the model's words. Latency is measured and printed: input
round trip through the orchestrator to the arbiter, and keystroke to changed tiles.
CUBEBOT_DESKTOP_EVIDENCE keeps the evidence directory, and CUBEBOT_DESKTOP_HOLD keeps a
failed run's sandbox and orchestrator for 15 minutes to inspect.
"""

import base64
import http.server
import json
import os
import platform
import secrets
import socket
import ssl
import statistics
import subprocess
import sys
import tempfile
import threading
import time
import traceback
import uuid
from pathlib import Path

from cluster_chat_setup import wait_for
from docker_sandbox import ROOT, Api, bridge, build, command, configure, live, remove
from docker_sandbox import sign_in as bootstrap
from guest_boot import rgb
from integration import run_database

MODEL = os.environ.get("CUBEBOT_COMPUTER_MODEL", "gpt-5.5")
WIDTH, HEIGHT = 1280, 800
CONSOLE = "http://127.0.0.1:3000"
MARKER = secrets.token_hex(4)
INSTRUCTIONS = (
    "You operate a Linux desktop through the computer tool. Do exactly the one action "
    "you are asked for, never retry a refused action, and reply with exactly the text "
    "the tool returned."
)
CLICK = "Click the middle of the screen once with the computer tool."


class Refused(Exception):
    pass


class Closed(Exception):
    pass


class Desktop:
    """A minimal RFC 6455 client: text frames only, which is all the desktop sends."""

    def __init__(self, address, ca, agent, headers):
        host, port = address.rsplit(":", 1)
        context = ssl.create_default_context(cafile=str(ca))
        raw = socket.create_connection((host, int(port)), timeout=30)
        self.socket = context.wrap_socket(raw, server_hostname=host)
        key = base64.b64encode(secrets.token_bytes(16)).decode()
        lines = [
            f"GET /api/agents/{agent}/desktop HTTP/1.1",
            f"Host: {address}",
            "Upgrade: websocket",
            "Connection: Upgrade",
            f"Sec-WebSocket-Key: {key}",
            "Sec-WebSocket-Version: 13",
            *(f"{name}: {value}" for name, value in headers.items()),
        ]
        self.socket.sendall(("\r\n".join(lines) + "\r\n\r\n").encode())
        self.buffer = b""
        while b"\r\n\r\n" not in self.buffer:
            self.buffer += self.recv()
        head, self.buffer = self.buffer.split(b"\r\n\r\n", 1)
        status = int(head.split()[1])
        if status != 101:
            self.socket.close()
            raise Refused(status)
        self.frames = []

    def recv(self):
        data = self.socket.recv(1 << 16)
        if not data:
            raise Closed(1006, "connection ended")
        return data

    def exactly(self, count):
        while len(self.buffer) < count:
            self.buffer += self.recv()
        data, self.buffer = self.buffer[:count], self.buffer[count:]
        return data

    def message(self):
        """The next text message; a close raises Closed with its code and reason."""
        while True:
            first, second = self.exactly(2)
            length = second & 0x7F
            if length == 126:
                length = int.from_bytes(self.exactly(2), "big")
            elif length == 127:
                length = int.from_bytes(self.exactly(8), "big")
            payload = self.exactly(length)
            opcode = first & 0x0F
            if opcode == 8:
                code = int.from_bytes(payload[:2], "big") if payload else 1005
                raise Closed(code, payload[2:].decode())
            if opcode == 1:
                return json.loads(payload)

    def send(self, value):
        payload = json.dumps(value).encode()
        mask = secrets.token_bytes(4)
        size = len(payload)
        if size < 126:
            header = bytes([0x81, 0x80 | size])
        else:
            header = bytes([0x81, 0x80 | 126]) + size.to_bytes(2, "big")
        masked = bytes(byte ^ mask[at % 4] for at, byte in enumerate(payload))
        self.socket.sendall(header + mask + masked)

    def frame(self):
        while True:
            message = self.message()
            if "ack" not in message:
                self.frames.append(message)
                return message

    def ask(self, message):
        """Sends a page message and returns its answer, keeping earlier frames."""
        t = time.monotonic() * 1000
        self.send(message | {"t": t})
        while True:
            answer = self.message()
            if answer.get("ack") == t:
                answer["elapsed"] = time.monotonic() * 1000 - t
                return answer
            self.frames.append(answer)

    def wait_mode(self, mode, timeout=15):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            if self.frame()["mode"] == mode:
                return
        raise AssertionError(f"the desktop did not reach mode {mode}")

    def closed(self, timeout):
        """The close the server sends within `timeout`."""
        self.socket.settimeout(timeout)
        try:
            while True:
                self.message()
        except Closed as closed:
            return closed.args
        finally:
            self.socket.settimeout(30)

    def close(self):
        self.socket.close()


class Person:
    def __init__(self, api, login, password, identity):
        self.api, self.login, self.password, self.id = api, login, password, identity

    def desktop(self, address, ca, agent):
        bearer = {"Authorization": f"Bearer {self.api.token}"}
        return Desktop(address, ca, agent, bearer)


def person(admin, base, ca, login, administrator=False):
    password = secrets.token_urlsafe(24)
    body = command(login=login, password=password, administrator=administrator)
    admin("/api/identity/accounts", body, expected=201)
    api = Api(base, ca)
    api.token = api("/api/identity/login", {"login": login, "password": password})[
        "token"
    ]
    return Person(api, login, password, api("/api/identity/me")["id"])


def grant(admin, account, agent, capability):
    body = command(
        recipient={"kind": "user", "id": account},
        scope={"kind": "agent", "id": agent},
        capability=capability,
    )
    return admin("/api/identity/access/grants", body, expected=201)


def agent(api, name, wake):
    identity = api("/api/agents", command(name=name, instructions=INSTRUCTIONS), 201)[
        "id"
    ]
    current = api(f"/api/agents/{identity}")
    update = command(
        expected_revision=current["revision"],
        name=name,
        instructions=INSTRUCTIONS,
        idle_policy=current["idle_policy"],
        idle_delay_seconds=86400,
        dreaming_model=None,
    )
    api(f"/api/agents/{identity}/definition", update)
    if wake:
        status = api(f"/api/agents/{identity}/status")
        body = command(expected_revision=status["revision"], action="wake")
        api(f"/api/agents/{identity}/lifecycle", body, expected=202)
    return identity


def container(identity):
    label = f"label=cubebot.dev/agent={identity}"
    found = subprocess.run(
        ["docker", "ps", "--quiet", "--filter", label],
        capture_output=True,
        text=True,
        check=True,
    ).stdout.split()
    return found[0] if found else None


def logs(identity):
    output = subprocess.run(
        ["docker", "logs", container(identity)], capture_output=True, text=True
    )
    return output.stdout + output.stderr


def arbiter_events(identity):
    return [
        json.loads(line.split("Desktop event ", 1)[1])
        for line in logs(identity).splitlines()
        if "Desktop event " in line
    ]


class Proof:
    """Requests the guest's Chromium makes here: what a person typed and clicked."""

    def __init__(self):
        self.paths = []
        paths = self.paths

        class Handler(http.server.BaseHTTPRequestHandler):
            def do_GET(self):
                paths.append(self.path)
                name = self.path.strip("/")
                link = f"/clicked-{name.removeprefix('page-')}"
                body = (
                    "<!doctype html><title>proof</title><body style='margin:0'>"
                    f"<a href='{link}' style='display:block;width:100vw;height:100vh;"
                    f"background:#c33'>{name}</a>"
                ).encode()
                self.send_response(200)
                self.send_header("Content-Type", "text/html")
                self.end_headers()
                self.wfile.write(body)

            def log_message(self, *arguments):
                pass

        self.server = http.server.ThreadingHTTPServer((bridge(), 0), Handler)
        self.origin = f"http://{bridge()}:{self.server.server_address[1]}"
        threading.Thread(target=self.server.serve_forever, daemon=True).start()

    def saw(self, path, timeout=30):
        wait_for(f"the guest to request {path}", lambda: path in self.paths, timeout)


def screen(frame):
    """The RGB screen a frame's tiles paint on black."""
    canvas = bytearray(WIDTH * HEIGHT * 3)
    for tile in frame["tiles"]:
        pixels = rgb(tile["png"])
        width = min(64, WIDTH - tile["x"])
        for row in range(len(pixels) // (width * 3)):
            start = ((tile["y"] + row) * WIDTH + tile["x"]) * 3
            canvas[start : start + width * 3] = pixels[
                row * width * 3 : (row + 1) * width * 3
            ]
    return bytes(canvas)


def similarity(first, second):
    same = sum(a == b for a, b in zip(first[::3], second[::3], strict=True))
    return same / (len(first) // 3)


def chat(api, identity, message):
    path = f"/api/conversations/{identity}/messages"
    turn = api(path, command(message=message), expected=202)["id"]

    def finished():
        row = next((row for row in api(path) if row["id"] == turn), None)
        return row if row and row["status"] in ("complete", "waiting") else None

    row = wait_for(f"the turn {message[:40]!r}", finished, timeout=600)
    print(f"  turn {turn}: {row['status']} {json.dumps(row['response'])[:200]}")
    return row


def refused(open_desktop):
    try:
        open_desktop().close()
    except Refused as refusal:
        return refusal.args[0]
    return 101


def scopes(world):
    """Who may open which desktop, and that a cookie needs the console's own origin."""
    address, ca, desk, idle = (
        world["address"],
        world["ca"],
        world["desk"],
        world["idle"],
    )
    people = world["people"]
    assert refused(lambda: Desktop(address, ca, desk, {})) == 401
    for name, expected in [("outsider", 403), ("viewer", 101), ("controller", 101)]:
        status = refused(lambda name=name: people[name].desktop(address, ca, desk))
        assert status == expected, (name, status)
    assert refused(lambda: people["viewer"].desktop(address, ca, idle)) == 403
    assert refused(lambda: people["admin"].desktop(address, ca, idle)) == 409
    cookie = {"Cookie": f"cubebot_session={people['viewer'].api.token}"}
    for origin, expected in [
        (None, 403),
        ("http://elsewhere.test", 403),
        (CONSOLE, 101),
    ]:
        headers = cookie | ({"Origin": origin} if origin else {})
        status = refused(lambda headers=headers: Desktop(address, ca, desk, headers))
        assert status == expected, (origin, status)
    print("PASS scopes: no credential 401, no grant 403, another agent 403, a stopped")
    print("     agent 409, a cookie only from the console's origin")


def watching(world):
    """A viewer sees the guest's real screen and can neither take control nor type."""
    viewer = world["people"]["viewer"].desktop(
        world["address"], world["ca"], world["desk"]
    )
    started = time.monotonic()
    first = viewer.frame()
    elapsed = (time.monotonic() - started) * 1000
    assert first["mode"] == "watch" and len(first["tiles"]) == 20 * 13, first["mode"]
    size = sum(len(tile["png"]) for tile in first["tiles"])
    shot = viewer.ask({"kind": "input", "action": {"action": "screenshot"}})
    match = similarity(screen(first), rgb(shot["png"]))
    assert match > 0.98, match
    assert len(set(screen(first))) > 8, "the first frame is blank"
    taken = viewer.ask({"kind": "take"})
    assert "permission" in taken["error"], taken
    click = {"action": "click", "at": [10, 10], "button": "left", "count": 1}
    typed = viewer.ask({"kind": "input", "action": click})
    assert typed["error"] == "this person does not have control", typed
    world["metrics"]["first_frame_ms"] = round(elapsed)
    world["metrics"]["first_frame_base64_kib"] = round(size / 1024)
    print(
        f"PASS watching: first frame of 260 tiles matches a screenshot ({match:.3f}),"
    )
    print("     the viewer cannot take control or send input")
    return viewer


def taking(world, viewer):
    people, address, ca, desk = (
        world[key] for key in ("people", "address", "ca", "desk")
    )
    controller = people["controller"].desktop(address, ca, desk)
    controller.frame()
    taken = controller.ask({"kind": "take"})
    assert taken["mode"] == "human" and "error" not in taken, taken
    viewer.wait_mode("human")
    admin = people["admin"].desktop(address, ca, desk)
    admin.frame()
    stolen = admin.ask({"kind": "take"})
    assert stolen["error"] == "another person has control", stolen
    admin.close()
    print("PASS control: the controller holds it, the viewer sees mode human, and an")
    print("     administrator cannot take it from them")
    return controller


def key(desktop, *keys):
    answer = desktop.ask(
        {"kind": "input", "action": {"action": "key", "keys": list(keys)}}
    )
    assert "error" not in answer, answer


def visit(desktop, url):
    # X starts with the keyboard focus off Chromium, as a person finds it: click first.
    bar = {"action": "click", "at": [600, 63], "button": "left", "count": 1}
    assert "error" not in desktop.ask({"kind": "input", "action": bar})
    key(desktop, "ctrl", "l")
    typed = desktop.ask({"kind": "input", "action": {"action": "type", "text": url}})
    assert "error" not in typed, typed
    key(desktop, "enter")


def human_input(world, controller, viewer):
    """The controller's keys and click reach the guest; the pointer is theirs."""
    proof = world["proof"]
    visit(controller, f"{proof.origin}/page-{MARKER}")
    proof.saw(f"/page-{MARKER}")
    time.sleep(2)
    click = {"action": "click", "at": [640, 400], "button": "left", "count": 1}
    assert "error" not in controller.ask({"kind": "input", "action": click})
    proof.saw(f"/clicked-{MARKER}")
    # The viewer's earlier frames are still queued on its socket.
    expected = {"at": [640, 400], "account": world["people"]["controller"].id}
    deadline = time.monotonic() + 15
    while viewer.frame()["pointer"] != expected:
        assert time.monotonic() < deadline, viewer.frames[-1]["pointer"]
    print("PASS human input: typed URL and click reached the guest's Chromium; the")
    print("     viewer sees the controller's pointer")


def agent_refused(world):
    """While a person holds control the agent's input is refused at the arbiter."""
    before = len(arbiter_events(world["desk"]))
    row = chat(world["people"]["admin"].api, world["desk"], CLICK)
    # The model is told of the refusal and answers; the conversation does not stall.
    assert row["status"] == "complete", row
    events = arbiter_events(world["desk"])[before:]
    inputs = [
        e
        for e in events
        if e["source"] == "agent" and e["action"]["action"] != "screenshot"
    ]
    assert inputs, events
    assert all(e.get("error") == "a person has control" for e in inputs), inputs
    outcomes = json.dumps([call["outcome"] for call in row["terminal"]])
    assert "a person has control" in outcomes, outcomes
    print(f"PASS agent refused: {len(inputs)} agent input(s) refused with")
    print("     'a person has control', and the ledger shows it")


def latency(world, controller):
    """Input round trips, and keystrokes to the frame that shows them."""
    trips = []
    for step in range(40):
        move = {"action": "move", "x": 100 + step, "y": 100}
        trips.append(controller.ask({"kind": "input", "action": move})["elapsed"])
    visit(controller, f"{world['proof'].origin}/latency-")
    time.sleep(2)
    key(controller, "ctrl", "l")
    pixels = []
    for character in "abcdefghijklmnopqrst":
        started = time.monotonic()
        controller.ask(
            {"kind": "input", "action": {"action": "type", "text": character}}
        )
        # Frames answered before the keystroke cannot show it.
        controller.frames.clear()
        while not any(frame.get("tiles") for frame in controller.frames):
            controller.frame()
        pixels.append((time.monotonic() - started) * 1000)
    key(controller, "escape")
    rates = []
    started, count = time.monotonic(), 0
    while time.monotonic() - started < 5:
        controller.frame()
        count += 1
    rates.append(count / 5)
    world["metrics"] |= {
        "input_round_trip_ms": summary(trips),
        "keystroke_to_tiles_ms": summary(pixels),
        "frames_per_second_idle": rates[0],
    }
    print("PASS latency measured:", json.dumps(world["metrics"]))


def summary(samples):
    ordered = sorted(samples)
    return {
        "median": round(statistics.median(ordered), 1),
        "p95": round(ordered[int(len(ordered) * 0.95) - 1], 1),
        "max": round(ordered[-1], 1),
        "n": len(ordered),
    }


def handing_back(world, controller, viewer):
    """After a hand-back the agent looks before acting; the transcript marks it."""
    answer = controller.ask({"kind": "hand_back"})
    assert answer["mode"] == "watch" and "error" not in answer, answer
    viewer.wait_mode("watch")
    before = len(arbiter_events(world["desk"]))
    row = chat(world["people"]["admin"].api, world["desk"], CLICK)
    assert row["status"] == "complete", row
    agent = [
        e for e in arbiter_events(world["desk"])[before:] if e["source"] == "agent"
    ]
    first = agent[0]
    assert first["action"]["action"] == "screenshot" or first.get(
        "error", ""
    ).startswith("a person had control"), first
    clicks = [e for e in agent if e["action"]["action"] == "click" and "error" not in e]
    assert clicks, agent
    history = world["people"]["admin"].api(
        f"/api/conversations/{world['desk']}/messages"
    )
    marked = [
        call for turn in history for call in turn["terminal"] if call["reobserved"]
    ]
    assert len(marked) == 1 and marked[0]["turn_id"] == row["id"], marked
    print(
        "PASS hand-back: the agent looked again before acting, its click went through,"
    )
    print("     and exactly its first screenshot after the hand-back is marked")


def disconnects(world, viewer):
    """A controller who vanishes, signs out or loses access gives control back."""
    people, address, ca, desk = (
        world[key] for key in ("people", "address", "ca", "desk")
    )
    dropped = people["controller"].desktop(address, ca, desk)
    dropped.frame()
    assert dropped.ask({"kind": "take"})["mode"] == "human"
    viewer.wait_mode("human")
    dropped.close()
    viewer.wait_mode("watch")
    print("PASS a dropped controller connection hands control back")

    held = people["controller"].desktop(address, ca, desk)
    held.frame()
    assert held.ask({"kind": "take"})["mode"] == "human"
    people["controller"].api("/api/identity/logout", {})
    code, reason = held.closed(45)
    assert reason == "access to this desktop ended", (code, reason)
    viewer.wait_mode("watch")
    print("PASS signing out ends the session within the recheck and hands control back")

    revoke = command(id=world["view_grant"]["id"], expected_revision=1)
    people["admin"].api("/api/identity/access/revoke", revoke)
    code, reason = viewer.closed(45)
    assert reason == "access to this desktop ended", (code, reason)
    print("PASS revoking browser.view ends a watching session within the recheck")


def recorded(world, postgres):
    """Sessions record viewers, starts, ends and control changes, and never pixels."""
    rows = psql(
        postgres,
        "SELECT a.login, jsonb_path_query_array(s.mode_changes,'$[*].mode')::text, "
        "s.ended_at IS NOT NULL FROM desktop_sessions s JOIN human_accounts a "
        f"ON a.id=s.viewer_id WHERE s.agent_id='{world['desk']}' ORDER BY s.started_at",
    )
    print("  desktop_sessions:", rows)
    sessions = [row.split("|") for row in rows]
    assert all(ended == "t" for *_, ended in sessions), sessions
    controller = [modes for login, modes, _ in sessions if login == "desk-controller"]
    assert (
        '["human", "watch"]' in controller
        and controller.count('["human", "watch"]') >= 2
    )
    stored = psql(
        postgres,
        "SELECT count(*) FROM desktop_sessions WHERE mode_changes::text LIKE '%iVBOR%'",
    )
    assert stored == ["0"], stored
    assert "iVBOR" not in world["orchestrator_log"].read_text()
    print(
        "PASS desktop_sessions holds every viewer, start, end and control change, and"
    )
    print("     neither it nor the orchestrator's log holds a frame")


def psql(postgres, query):
    return subprocess.run(
        [
            "docker",
            "exec",
            postgres,
            "psql",
            "-U",
            "cubebot",
            "-d",
            "cubebot_test",
            "-Atc",
            query,
        ],
        capture_output=True,
        text=True,
        check=True,
    ).stdout.splitlines()


def console(world):
    """In Chromium a person watches, takes control, types, clicks and hands back.

    The console lists only agents a person manages, so this is the administrator.
    """
    environment = os.environ | {
        "CUBEBOT_CONSOLE": CONSOLE,
        "CUBEBOT_CONSOLE_LOGIN": "desk-admin",
        "CUBEBOT_CONSOLE_PASSWORD": world["people"]["admin"].password,
        "CUBEBOT_PROOF_URL": f"{world['proof'].origin}/page-ui{MARKER}",
        "CUBEBOT_EVIDENCE_DIRECTORY": str(world["evidence"]),
    }
    script = Path(__file__).with_name("docker_desktop_browser.mjs")
    result = subprocess.run(
        ["node", script], env=environment, capture_output=True, text=True, timeout=600
    )
    print(result.stdout + result.stderr)
    assert result.returncode == 0, "the console check failed"
    world["proof"].saw(f"/page-ui{MARKER}")
    world["proof"].saw(f"/clicked-ui{MARKER}")
    reported = json.loads(result.stdout.strip().splitlines()[-1])
    world["metrics"]["console_reported"] = reported
    print(
        "PASS console: the Desktop tab painted the guest, took control, its typed URL"
    )
    print("     and click reached Chromium, it handed back, and the marker shows")


def surroundings(world, address, ca):
    """The built console, and Caddy before it and the orchestrator as an ingress."""
    caddyfile = world["evidence"] / "Caddyfile"
    caddyfile.write_text(
        f"{CONSOLE} {{\n\treverse_proxy /api/* https://{address} {{\n"
        "\t\ttransport http {\n\t\t\ttls_trusted_ca_certs /ca.crt\n\t\t}\n\t}\n"
        "\treverse_proxy 127.0.0.1:3001\n}\n"
    )
    names = [f"cubebot-desktop-web-{MARKER}", f"cubebot-desktop-caddy-{MARKER}"]
    subprocess.run(
        ["docker", "run", "-d", "--rm", "--name", names[0], "--network", "host"]
        + ["-e", "PORT=3001", "-e", "HOST=127.0.0.1", "cubebot-web:local"],
        check=True,
        stdout=subprocess.DEVNULL,
    )
    subprocess.run(
        ["docker", "run", "-d", "--rm", "--name", names[1], "--network", "host"]
        + ["-v", f"{caddyfile}:/etc/caddy/Caddyfile:ro", "-v", f"{ca}:/ca.crt:ro"]
        + ["caddy:2.10.2-alpine"],
        check=True,
        stdout=subprocess.DEVNULL,
    )
    return names


def host_frame(desk):
    request = b'"frame"\n'
    relay = subprocess.run(
        ["docker", "exec", "-i", container(desk), "cubebot-sandbox", "desktop"],
        input=request,
        capture_output=True,
        check=True,
        timeout=60,
    )
    return json.loads(relay.stdout)


def ready(people, desk):
    status = f"/api/agents/{desk}/status"
    wait_for(
        "Desk's runtime", lambda: people["admin"].api(status)["runtime_available"], 300
    )
    wait_for("Desk's guest desktop", lambda: "Guest desktop started" in logs(desk), 120)
    # Chromium paints a while after the desktop starts.
    wait_for(
        "Chromium to paint the guest screen",
        lambda: len(set(screen(host_frame(desk)))) > 8,
        120,
    )
    time.sleep(5)
    # The host channel, through the exec the orchestrator uses, answers each request
    # while it stays open, as the orchestrator holds it.
    relay = subprocess.Popen(
        ["docker", "exec", "-i", container(desk), "cubebot-sandbox", "desktop"],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
    )
    answers = []
    for _ in range(2):
        started = time.monotonic()
        relay.stdin.write(b'"frame"\n')
        relay.stdin.flush()
        line = relay.stdout.readline()
        answers.append(json.loads(line))
        print(f"  host channel: {len(line)} bytes, {time.monotonic() - started:.3f}s")
    relay.kill()
    first, second = answers
    assert len(first["tiles"]) == 20 * 13 and "tiles" not in second, second


def setup(admin, base, ca, password):
    admin_id = admin("/api/identity/me")["id"]
    key = Path(os.environ["OPENAI_API_KEY_FILE"]).read_text().strip()
    provider = command(
        expected_revision=0,
        api_key=key,
        model=MODEL,
        provider="openai",
        context_window=128000,
    )
    admin("/api/provider", provider)
    desk, idle = agent(admin, "Desk", True), agent(admin, "Idle", False)
    people = {"admin": Person(admin, "desk-admin", password, admin_id)}
    for name in ["viewer", "controller", "outsider"]:
        people[name] = person(admin, base, ca, f"desk-{name}")
    view = grant(admin, people["viewer"].id, desk, "browser_view")
    grant(admin, people["controller"].id, desk, "browser_view")
    grant(admin, people["controller"].id, desk, "browser_control")
    return {"people": people, "desk": desk, "idle": idle, "view_grant": view}


def scenarios(world, postgres):
    scopes(world)
    viewer = watching(world)
    controller = taking(world, viewer)
    human_input(world, controller, viewer)
    agent_refused(world)
    latency(world, controller)
    handing_back(world, controller, viewer)
    controller.close()
    console(world)
    disconnects(world, viewer)
    viewer.close()
    time.sleep(2)
    recorded(world, postgres)
    print("METRICS", json.dumps(world["metrics"]))


def qualify(directory: Path):
    with socket.socket() as probe:
        probe.bind((bridge(), 0))
        address = f"{bridge()}:{probe.getsockname()[1]}"
    environment = configure(directory, address)
    config = directory / "docker.json"
    config.write_text(
        json.dumps(json.loads(config.read_text()) | {"computer_use_enabled": True})
    )
    environment["CUBEBOT_PUBLIC_ORIGIN"] = CONSOLE
    base, ca = f"https://{address}", directory / "ca/ca.crt"
    admin = Api(base, ca)
    world = {"address": address, "ca": ca, "evidence": directory, "metrics": {}}
    world["orchestrator_log"] = directory / "orchestrator.log"

    def workflow(postgres, env):
        environment["CUBEBOT_DATABASE_URL"] = env["CUBEBOT_TEST_DATABASE_URL"]
        binary = [str(ROOT / "target/debug/cubebot-orchestrator")]
        with open(world["orchestrator_log"], "w") as log:
            process = subprocess.Popen(
                binary,
                env=os.environ | environment,
                stdout=log,
                stderr=subprocess.STDOUT,
            )
        names = []
        try:
            wait_for("orchestrator HTTPS", lambda: live(admin), timeout=30)
            password = secrets.token_urlsafe(24)
            bootstrap(admin, env["CUBEBOT_TEST_DATABASE_URL"], "desk-admin", password)
            world.update(setup(admin, base, ca, password))
            names = surroundings(world, address, ca)
            ready(world["people"], world["desk"])
            world["proof"] = Proof()
            scenarios(world, postgres)
        except BaseException:
            print(world["orchestrator_log"].read_text()[-20000:])
            for identity in (world.get("desk"),):
                if identity and container(identity):
                    print(logs(identity)[-20000:])
            if os.environ.get("CUBEBOT_DESKTOP_HOLD"):
                traceback.print_exc()
                print("HOLDING for inspection", flush=True)
                time.sleep(900)
            raise
        finally:
            subprocess.run(
                ["docker", "rm", "-f", *names], check=False, capture_output=True
            )
            for identity in (world.get("desk"), world.get("idle")):
                if identity:
                    remove(identity)
            process.terminate()
            process.wait(timeout=30)
        print("PASS the desktop view on real agents, from every person's side")

    with tempfile.TemporaryDirectory(
        prefix="cubebot-postgres-", dir=ROOT / "target"
    ) as data:
        password = secrets.token_hex(24)
        name = f"cubebot-test-{uuid.uuid4().hex}"
        env = os.environ | {"POSTGRES_PASSWORD": password}
        run_database(name, env, password, os.path.abspath(data), workflow)


def main():
    if sys.platform != "linux" or platform.machine() != "x86_64":
        print(f"docker-desktop skipped: needs x86_64 Linux, this is {sys.platform}")
        return 0
    if not os.environ.get("OPENAI_API_KEY_FILE"):
        print("docker-desktop failed: set OPENAI_API_KEY_FILE", file=sys.stderr)
        return 1
    build()
    subprocess.run(
        [
            "docker",
            "build",
            "--tag",
            "cubebot-web:local",
            "--file",
            "apps/web/Dockerfile",
            ".",
        ],
        check=True,
        cwd=ROOT,
    )
    keep = os.environ.get("CUBEBOT_DESKTOP_EVIDENCE")
    with tempfile.TemporaryDirectory() as directory:
        qualify(Path(keep or directory))
    return 0


if __name__ == "__main__":
    sys.exit(main())
