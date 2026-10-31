"""Login handoff (ADRs 0039 and 0085) on a real agent through Docker.

Same host needs as docker_desktop.py, Node with Playwright included. Keeper runs the
guest desktop with OpenAI's model and asks for logins with request_login; a controller
with `browser.view` and `browser.control`, a viewer with `browser.view` only and an
outsider with no grant each see their own side of it. The person's login is proved where
it lands, on a server this script runs, and the agent's refusals in the arbiter's log
and the ledger, never in the model's words. CUBEBOT_LOGIN_EVIDENCE keeps the evidence
directory (console screenshots); CUBEBOT_LOGIN_HOLD keeps a failed run for 15 minutes.
"""

import http.server
import json
import os
import platform
import secrets
import socket
import subprocess
import sys
import tempfile
import threading
import time
import traceback
import urllib.parse
import uuid
from pathlib import Path

from cluster_chat_setup import wait_for
from docker_browser import calls, chat, lifecycle, ready
from docker_desktop import (
    CONSOLE,
    MODEL,
    Person,
    arbiter_events,
    container,
    grant,
    logs,
    person,
    psql,
    surroundings,
    visit,
)
from docker_sandbox import ROOT, Api, bridge, build, command, configure, live, remove
from docker_sandbox import sign_in as bootstrap
from integration import run_database

MARKER = secrets.token_hex(4)
SECRET = f"pw-{secrets.token_hex(8)}"
PENDING = "a person is logging in for you"
INSTRUCTIONS = (
    "You use the browser_* tools for web pages and the computer tool for the screen. "
    "Do exactly the steps you are asked for, in order, never retry a refused call, "
    "and reply with a short note of what happened."
)


class Site:
    """A login page; a session cookie once a person submits it; an account page."""

    def __init__(self):
        self.seen = []
        seen = self.seen

        class Handler(http.server.BaseHTTPRequestHandler):
            def do_GET(self):
                seen.append((self.path, self.headers.get("Cookie", "")))
                parsed = urllib.parse.urlparse(self.path)
                body, cookie = page(parsed.path.strip("/"), self.headers)
                self.send_response(200)
                self.send_header("Content-Type", "text/html")
                if cookie:
                    self.send_header("Set-Cookie", cookie)
                self.end_headers()
                self.wfile.write(body.encode())

            def log_message(self, *arguments):
                pass

        self.server = http.server.ThreadingHTTPServer((bridge(), 0), Handler)
        self.host = f"{bridge()}:{self.server.server_address[1]}"
        threading.Thread(target=self.server.serve_forever, daemon=True).start()

    def url(self, path):
        return f"http://{self.host}/{path}"

    def requests(self, path):
        return [
            (p, c) for p, c in self.seen if urllib.parse.urlparse(p).path == f"/{path}"
        ]

    def saw(self, path, timeout=30):
        return wait_for(
            f"the guest to request /{path}", lambda: self.requests(path), timeout
        )


def page(path, headers):
    if path == f"login-{MARKER}":
        return (
            f"<!doctype html><title>Sign in {MARKER}</title><h1>Sign in</h1>"
            f"<form action='/session-{MARKER}'>"
            "<label>User <input name=user autofocus></label> "
            "<label>Password <input name=password type=password></label> "
            "<button>Sign in</button></form>",
            None,
        )
    if path == f"session-{MARKER}":
        cookie = f"session={MARKER}; Max-Age=86400; Path=/"
        return f"<!doctype html><title>Welcome {MARKER}</title><h1>Welcome</h1>", cookie
    if path == f"account-{MARKER}":
        signed = f"session={MARKER}" in (headers.get("Cookie") or "")
        title = "Account" if signed else "Please sign in"
        return f"<!doctype html><title>{title} {MARKER}</title><h1>{title}</h1>", None
    return f"<!doctype html><title>{path}</title>", None


def pending(world, postgres):
    rows = psql(
        postgres,
        "SELECT site, status, coalesce(taken_by::text,''), "
        "coalesce(settled_by::text,''), coalesce(declined_because,'') "
        f"FROM login_handoffs WHERE agent_id='{world['keeper']}' ORDER BY created_at",
    )
    return [row.split("|") for row in rows]


def banner(who, world, expected=200):
    return who.api(f"/api/agents/{world['keeper']}/login", expected=expected)


def modes(identity):
    return [
        json.loads(line.split("Desktop mode ", 1)[1])
        for line in logs(identity).splitlines()
        if "Desktop mode " in line
    ]


def turns(world):
    admin = world["people"]["admin"].api
    return admin(f"/api/conversations/{world['keeper']}/messages")


def request(world, site, reason):
    """The model asks for a login and its turn ends with that call."""
    admin, keeper = world["people"]["admin"].api, world["keeper"]
    row = chat(
        admin,
        keeper,
        f"Call request_login with site {site!r} and reason {reason!r}, then end "
        "your turn without any other tool call.",
    )
    asked = calls(row, "request_login")
    assert row["status"] == "complete", row
    assert row["terminal"][-1]["command"].startswith("request_login "), row["terminal"]
    return row, asked


def scopes_before(world):
    people, keeper = world["people"], world["keeper"]
    assert banner(people["viewer"], world, 204) is None
    banner(people["outsider"], world, 403)
    Api(world["base"], world["ca"])(f"/api/agents/{keeper}/login", expected=401)
    print("PASS scopes: no pending login shows nothing to a viewer; an outsider 403,")
    print("     no credential 401")


def requested(world, postgres):
    """A real model opens the login page and asks a person; the turn ends."""
    admin, keeper, site = world["people"]["admin"].api, world["keeper"], world["site"]
    row = chat(
        admin,
        keeper,
        f"In browser session main, browser_navigate to {site.url(f'login-{MARKER}')}. "
        "It is a login page you must not fill in yourself: then call request_login "
        f"with site {site.host!r} and reason 'Read the account page', and end your "
        "turn.",
    )
    site.saw(f"login-{MARKER}")
    asked = calls(row, "request_login")
    assert len(asked) == 1 and asked[0]["outcome"]["code"] == 0, asked
    assert row["terminal"][-1]["command"].startswith("request_login "), row["terminal"]
    assert row["status"] == "complete", row
    assert pending(world, postgres) == [[site.host, "pending", "", "", ""]], pending(
        world, postgres
    )
    for name in ("viewer", "controller", "admin"):
        shown = banner(world["people"][name], world)
        assert shown["site"] == site.host, (name, shown)
        assert shown["reason"] == "Read the account page", (name, shown)
    banner(world["people"]["outsider"], world, 403)
    world["login_turn"] = row["id"]
    print("PASS request: the model navigated to the login page, called request_login")
    print("     as its last call and ended the turn; viewer, controller and admin see")
    print("     the banner, the outsider does not")


def duplicate(world, postgres):
    row, asked = request(world, "other.example", "A second login")
    outcome = asked[0]["outcome"]
    assert outcome["code"] == 1 and "already waiting" in outcome["output"], outcome
    assert len(pending(world, postgres)) == 1, pending(world, postgres)
    print("PASS one at a time: a second request while one is pending is refused with")
    print("     the reason, and records nothing")


def permissions(world):
    people, address, ca, keeper = (
        world[key] for key in ("people", "address", "ca", "keeper")
    )
    decline = {"reason": "no"}
    path = f"/api/agents/{keeper}/login/decline"
    people["viewer"].api(path, decline, expected=403)
    people["outsider"].api(path, decline, expected=403)
    viewer = people["viewer"].desktop(address, ca, keeper)
    viewer.frame()
    assert "permission" in viewer.ask({"kind": "take"})["error"]
    viewer.close()
    print("PASS permissions: a viewer and an outsider cannot decline, and a viewer")
    print("     cannot take control")


def take(world, postgres):
    people, address, ca, keeper = (
        world[key] for key in ("people", "address", "ca", "keeper")
    )
    controller = people["controller"].desktop(address, ca, keeper)
    controller.frame()
    taken = controller.ask({"kind": "take"})
    assert taken["mode"] == "human" and "error" not in taken, taken
    last = modes(keeper)[-1]
    assert last == {
        "mode": "human",
        "account": people["controller"].id,
        "handoff": True,
    }, last
    taker = pending(world, postgres)[0][2]
    assert taker == people["controller"].id, taker
    print("PASS take: the controller took control for the handoff, the arbiter knows")
    print("     it is a login handoff, and the row names the controller")
    return controller


def blocked(world):
    """While the login is in progress the agent sees and drives nothing."""
    admin, keeper = world["people"]["admin"].api, world["keeper"]
    before = len(arbiter_events(keeper))
    row = chat(
        admin,
        keeper,
        "Take a screenshot with the computer tool, then call browser_read for "
        "browser session main.",
    )
    assert row["status"] == "complete", row
    events = [e for e in arbiter_events(keeper)[before:] if e["source"] == "agent"]
    assert events and all(PENDING in e.get("error", "") for e in events), events
    shots = [e for e in events if e["action"]["action"] == "screenshot"]
    asks = [e for e in events if e["action"]["action"] == "mode"]
    assert shots and asks, events
    read = calls(row, "browser_read")
    assert all(PENDING in call["outcome"]["output"] for call in read), read
    for call in row["terminal"]:
        assert call["outcome"]["status"] != "screenshot", call
    print(f"PASS blocked: {len(shots)} screenshot(s) and {len(asks)} browser call(s)")
    print("     refused at the arbiter with the pending login named; no image reached")
    print("     the ledger")


def login_by_hand(world, controller):
    """The controller signs in on the guest's Chromium with the keyboard."""
    site = world["site"]
    visit(controller, site.url(f"login-{MARKER}"))
    time.sleep(3)
    for action in (
        {"action": "type", "text": "ada"},
        {"action": "key", "keys": ["tab"]},
        {"action": "type", "text": SECRET},
        {"action": "key", "keys": ["enter"]},
    ):
        answer = controller.ask({"kind": "input", "action": action})
        assert "error" not in answer, answer
    path, _ = site.saw(f"session-{MARKER}")[-1]
    query = urllib.parse.parse_qs(urllib.parse.urlparse(path).query)
    assert query == {"user": ["ada"], "password": [SECRET]}, path
    print("PASS login: the controller's typing signed in on the site")


def dropped(world, postgres, controller):
    """A lost connection is not a hand-back: the login stays pending."""
    keeper = world["keeper"]
    controller.close()
    wait_for(
        "the arbiter to leave human mode",
        lambda: modes(keeper)[-1]["mode"] != "human",
        30,
    )
    assert modes(keeper)[-1]["handoff"] is False, modes(keeper)[-1]
    assert pending(world, postgres)[0][1] == "pending", pending(world, postgres)
    assert banner(world["people"]["viewer"], world)["site"] == world["site"].host
    count = len(turns(world))
    time.sleep(5)
    assert len(turns(world)) == count, "a dropped connection queued a turn"
    print("PASS dropped: closing the connection without handing back left the login")
    print("     pending, queued nothing, and cleared the arbiter's handoff")


def completed(world, postgres):
    """Handing back completes it; the next turn reads it and looks again."""
    people, address, ca, keeper, site = (
        world[key] for key in ("people", "address", "ca", "keeper", "site")
    )
    controller = people["controller"].desktop(address, ca, keeper)
    controller.frame()
    assert controller.ask({"kind": "take"})["mode"] == "human"
    assert modes(keeper)[-1]["handoff"] is True
    before = len(arbiter_events(keeper))
    answer = controller.ask({"kind": "hand_back"})
    assert answer["mode"] == "watch" and "error" not in answer, answer
    controller.close()
    message = f"Login completed on {site.host}, take a screenshot."

    def done():
        row = next((row for row in turns(world) if row["message"] == message), None)
        return row if row and row["status"] == "complete" else None

    row = wait_for("the completion turn", done, 600)
    print(f"  turn {row['id']}: {json.dumps(row['response'])[:200]}")
    shots = [c for c in row["terminal"] if c["outcome"]["status"] == "screenshot"]
    assert shots, row["terminal"]
    events = [e for e in arbiter_events(keeper)[before:] if e["source"] == "agent"]
    assert not any(PENDING in e.get("error", "") for e in events), events
    rows = pending(world, postgres)
    controller_id = people["controller"].id
    assert rows[0][1:4] == ["completed", controller_id, controller_id], rows
    assert banner(people["viewer"], world, 204) is None
    account = chat(
        people["admin"].api,
        keeper,
        f"In browser session main, browser_navigate to {site.url(f'account-{MARKER}')} "
        "and then browser_read it.",
    )
    _, cookies = site.saw(f"account-{MARKER}")[-1]
    assert f"session={MARKER}" in cookies, cookies
    tree = calls(account, "browser_read")[0]["outcome"]["output"]
    assert f"Account {MARKER}" in tree, tree
    print("PASS complete: hand-back completed the login, the next turn read it and")
    print("     took a screenshot that went through, and the agent's browser carries")
    print("     the session the person signed in with")


def secrets_kept(world, postgres):
    for table in ("turn_events", "chat_turns", "login_handoffs", "desktop_sessions"):
        found = psql(
            postgres,
            f"SELECT count(*) FROM {table} t WHERE t::text LIKE '%{SECRET}%'",
        )
        assert found == ["0"], (table, found)
    assert SECRET not in world["orchestrator_log"].read_text()
    assert SECRET not in logs(world["keeper"])
    print("PASS secrets: the password the person typed is in no table, the")
    print("     orchestrator's log or the sandbox's log")


def declined(world, postgres):
    people, keeper = world["people"], world["keeper"]
    request(world, "mail.example", "Read the inbox")
    path = f"/api/agents/{keeper}/login/decline"
    reason = "Use the API token instead"
    people["controller"].api(path, {"reason": reason}, expected=204)
    people["controller"].api(path, {"reason": reason}, expected=404)
    message = f"A person declined to log in on mail.example: {reason}"

    def done():
        row = next((row for row in turns(world) if row["message"] == message), None)
        return row if row and row["status"] == "complete" else None

    row = wait_for("the decline turn", done, 600)
    print(f"  turn {row['id']}: {json.dumps(row['response'])[:200]}")
    rows = pending(world, postgres)
    assert rows[-1] == [
        "mail.example",
        "declined",
        "",
        people["controller"].id,
        reason,
    ], rows
    assert banner(people["viewer"], world, 204) is None
    print("PASS decline: the controller declined with a reason, the agent's next turn")
    print("     carries it, and declining again is 404")


def console(world, postgres, step):
    """The banner in real Chromium: take control and hand back, or decline."""
    site = "console.example" if step == "take" else "decline.example"
    request(world, site, f"Console {step}")
    environment = os.environ | {
        "CUBEBOT_CONSOLE": CONSOLE,
        "CUBEBOT_CONSOLE_LOGIN": "login-admin",
        "CUBEBOT_CONSOLE_PASSWORD": world["people"]["admin"].password,
        "CUBEBOT_EVIDENCE_DIRECTORY": str(world["evidence"]),
        "CUBEBOT_LOGIN_SITE": site,
        "CUBEBOT_LOGIN_STEP": step,
    }
    script = Path(__file__).with_name("docker_login_browser.mjs")
    result = subprocess.run(
        ["node", script], env=environment, capture_output=True, text=True, timeout=600
    )
    print(result.stdout + result.stderr)
    assert result.returncode == 0, "the console check failed"
    status = "completed" if step == "take" else "declined"
    assert pending(world, postgres)[-1][:2] == [site, status], pending(world, postgres)
    print(f"PASS console {step}: the banner in the chat and the Desktop tab settled it")


def scenarios(world, postgres):
    scopes_before(world)
    requested(world, postgres)
    duplicate(world, postgres)
    permissions(world)
    controller = take(world, postgres)
    blocked(world)
    login_by_hand(world, controller)
    dropped(world, postgres, controller)
    completed(world, postgres)
    secrets_kept(world, postgres)
    declined(world, postgres)
    console(world, postgres, "take")
    console(world, postgres, "decline")


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
    body = command(name="Keeper", instructions=INSTRUCTIONS)
    keeper = admin("/api/agents", body, 201)["id"]
    current = admin(f"/api/agents/{keeper}")
    admin(
        f"/api/agents/{keeper}/definition",
        command(
            expected_revision=current["revision"],
            name="Keeper",
            instructions=INSTRUCTIONS,
            idle_policy=current["idle_policy"],
            idle_delay_seconds=86400,
            dreaming_model=None,
        ),
    )
    lifecycle(admin, keeper, "wake")
    people = {"admin": Person(admin, "login-admin", password, admin_id)}
    for name in ["viewer", "controller", "outsider"]:
        people[name] = person(admin, base, ca, f"login-{name}")
    grant(admin, people["viewer"].id, keeper, "browser_view")
    grant(admin, people["controller"].id, keeper, "browser_view")
    grant(admin, people["controller"].id, keeper, "browser_control")
    return {"people": people, "keeper": keeper}


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
    world = {"address": address, "base": base, "ca": ca, "evidence": directory}
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
            bootstrap(admin, env["CUBEBOT_TEST_DATABASE_URL"], "login-admin", password)
            world.update(setup(admin, base, ca, password))
            names = surroundings(world, address, ca)
            ready(admin, world["keeper"])
            world["site"] = Site()
            scenarios(world, postgres)
        except BaseException:
            print(world["orchestrator_log"].read_text()[-20000:])
            keeper = world.get("keeper")
            if keeper and container(keeper):
                print(logs(keeper)[-20000:])
            if os.environ.get("CUBEBOT_LOGIN_HOLD"):
                traceback.print_exc()
                print("HOLDING for inspection", flush=True)
                time.sleep(900)
            raise
        finally:
            subprocess.run(
                ["docker", "rm", "-f", *names], check=False, capture_output=True
            )
            if world.get("keeper"):
                remove(world["keeper"])
            process.terminate()
            process.wait(timeout=30)
        print("PASS login handoff on a real agent, from every person's side")

    with tempfile.TemporaryDirectory(
        prefix="cubebot-postgres-", dir=ROOT / "target"
    ) as data:
        password = secrets.token_hex(24)
        name = f"cubebot-test-{uuid.uuid4().hex}"
        env = os.environ | {"POSTGRES_PASSWORD": password}
        run_database(name, env, password, os.path.abspath(data), workflow)


def main():
    if sys.platform != "linux" or platform.machine() != "x86_64":
        print(f"docker-login skipped: needs x86_64 Linux, this is {sys.platform}")
        return 0
    if not os.environ.get("OPENAI_API_KEY_FILE"):
        print("docker-login failed: set OPENAI_API_KEY_FILE", file=sys.stderr)
        return 1
    build()
    subprocess.run(
        ["docker", "build", "--tag", "cubebot-web:local"]
        + ["--file", "apps/web/Dockerfile", "."],
        check=True,
        cwd=ROOT,
    )
    keep = os.environ.get("CUBEBOT_LOGIN_EVIDENCE")
    with tempfile.TemporaryDirectory() as directory:
        qualify(Path(keep or directory))
    return 0


if __name__ == "__main__":
    sys.exit(main())
