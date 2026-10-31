"""The agent's browser tools (ADRs 0036, 0037 and 0085) on real agents through Docker.

Same host needs as docker_sandbox.py. Two agents with computer use drive the guest's
desktop Chromium through the browser tools with OpenAI's model: Surfer in its main
session, Tasks in named task sessions. Every claim is read from evidence the model
cannot write: requests and cookies the guest's Chromium sends to a server this script
runs, the ledger's tool calls and outcomes, the arbiter's log, and the host's own frame
of the guest screen. Nothing is taken from the model's words. CUBEBOT_BROWSER_HOLD keeps
a failed run alive for 15 minutes.
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
from docker_desktop import (
    MODEL,
    arbiter_events,
    container,
    grant,
    host_frame,
    logs,
    person,
    psql,
    screen,
)
from docker_sandbox import ROOT, Api, bridge, build, command, configure, live, remove
from docker_sandbox import sign_in as bootstrap
from integration import run_database

MARKER = secrets.token_hex(4)
NAME = f"Ada {MARKER}"
INSTRUCTIONS = (
    "You browse the web only with the browser_* tools, never the computer tool. Use "
    "exactly the session names you are given. Do exactly the steps you are asked for, "
    "in order, never retry a refused call, and reply with the last tool's output."
)
RED = (0xCC, 0x33, 0x33)


class Site:
    """The pages the guest's Chromium visits, and every request with its cookies."""

    def __init__(self):
        self.seen = []
        seen = self.seen

        class Handler(http.server.BaseHTTPRequestHandler):
            def do_GET(self):
                seen.append((self.path, self.headers.get("Cookie", "")))
                path = urllib.parse.urlparse(self.path).path.strip("/")
                body, cookie = page(path)
                self.send_response(200)
                self.send_header("Content-Type", "text/html")
                if cookie:
                    self.send_header("Set-Cookie", cookie)
                self.end_headers()
                self.wfile.write(body.encode())

            def log_message(self, *arguments):
                pass

        self.server = http.server.ThreadingHTTPServer((bridge(), 0), Handler)
        self.origin = f"http://{bridge()}:{self.server.server_address[1]}"
        threading.Thread(target=self.server.serve_forever, daemon=True).start()

    def url(self, path):
        return f"{self.origin}/{path}"

    def requests(self, path):
        return [
            (p, c) for p, c in self.seen if urllib.parse.urlparse(p).path == f"/{path}"
        ]

    def saw(self, path, timeout=30):
        return wait_for(
            f"the guest to request /{path}", lambda: self.requests(path), timeout
        )


def page(path):
    """A page's HTML and the cookie it sets; `set-<name>` sets `<name>=MARKER`."""
    if path == f"form-{MARKER}":
        return (
            f"<!doctype html><title>Form {MARKER}</title>"
            f"<form action='/submitted-{MARKER}'>"
            "<label>Full name <input name=name></label> <button>Send</button></form>",
            None,
        )
    if path == f"red-{MARKER}":
        return (
            "<!doctype html><title>Red</title>"
            "<body style='margin:0;background:#cc3333;height:100vh'></body>",
            None,
        )
    if path.startswith("set-"):
        name = path.removeprefix("set-")
        cookie = f"{name}={MARKER}; Max-Age=86400; Path=/"
        return f"<!doctype html><title>Set {name}</title><p>set {name}</p>", cookie
    return f"<!doctype html><title>{path}</title><p>{path}</p>", None


def agent(api, name):
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
    lifecycle(api, identity, "wake")
    return identity


def lifecycle(api, identity, action):
    status = api(f"/api/agents/{identity}/status")
    body = command(expected_revision=status["revision"], action=action)
    api(f"/api/agents/{identity}/lifecycle", body, expected=202)


def ready(api, identity):
    """The runtime is up and its sandbox's desktop has started."""
    status = f"/api/agents/{identity}/status"
    wait_for("the runtime", lambda: api(status)["runtime_available"], 300)
    wait_for(
        "the guest desktop", lambda: "Guest desktop started" in logs(identity), 180
    )
    # Chromium opens its DevTools port a while after the desktop starts.
    wait_for(
        "Chromium to paint", lambda: len(set(screen(host_frame(identity)))) > 8, 120
    )
    time.sleep(5)


def chat(api, identity, message):
    """A turn that called tools; a reply without a call is asked again once, since a
    model can answer from earlier results instead of acting."""
    row = turn(api, identity, message)
    if not row["terminal"]:
        print("  the model answered without a tool call; asking again")
        row = turn(api, identity, f"You did not call a tool. Call it now: {message}")
    return row


def turn(api, identity, message):
    path = f"/api/conversations/{identity}/messages"
    turn = api(path, command(message=message), expected=202)["id"]

    def finished():
        row = next((row for row in api(path) if row["id"] == turn), None)
        return row if row and row["status"] in ("complete", "waiting") else None

    row = wait_for(f"the turn {message[:50]!r}", finished, timeout=600)
    print(f"  turn {turn}: {row['status']} {json.dumps(row['response'])[:160]}")
    for call in row["terminal"]:
        print(f"    {call['command'][:110]} -> {json.dumps(call['outcome'])[:140]}")
    return row


def calls(row, tool):
    """The turn's calls of one tool, each with its parsed input."""
    found = []
    for call in row["terminal"]:
        name, _, rest = call["command"].partition(" ")
        if name == tool:
            found.append(call | {"input": json.loads(rest)})
    assert found, (tool, [call["command"] for call in row["terminal"]])
    return found


def succeeded(call):
    outcome = call["outcome"]
    return outcome["status"] == "screenshot" or (
        outcome["status"] == "exited" and outcome.get("code") == 0
    )


def mode_checks(identity, row):
    """The arbiter's mode queries made for this turn's calls, by call id."""
    ids = {call["id"] for call in row["terminal"]}
    return [
        event
        for event in arbiter_events(identity)
        if event["source"] == "agent"
        and event["action"]["action"] == "mode"
        and event.get("call") in ids
    ]


def form(world):
    """Navigate, read, type and click in the main session submit a real form."""
    api, surfer, site = world["api"], world["surfer"], world["site"]
    row = chat(
        api,
        surfer,
        f"In browser session main: browser_navigate to {site.url(f'form-{MARKER}')}, "
        f"then browser_read, then browser_type the text {NAME!r} into the Full name "
        "field, then browser_click the Send button.",
    )
    path, _ = site.saw(f"submitted-{MARKER}")[0]
    query = urllib.parse.parse_qs(urllib.parse.urlparse(path).query)
    assert query == {"name": [NAME]}, path
    tree = calls(row, "browser_read")[0]["outcome"]["output"]
    assert f'RootWebArea "Form {MARKER}"' in tree and "textbox" in tree, tree
    for tool in ["browser_navigate", "browser_read", "browser_type", "browser_click"]:
        assert all(succeeded(call) for call in calls(row, tool)), tool
        assert all(call["input"]["session"] == "main" for call in calls(row, tool))
    checked = {event["call"] for event in mode_checks(surfer, row)}
    inputs = {
        call["id"]
        for tool in ["browser_navigate", "browser_type", "browser_click"]
        for call in calls(row, tool)
    }
    reads = {call["id"] for call in calls(row, "browser_read")}
    # Every call asks, so a login handoff (ADR 0039) can refuse reads too.
    assert inputs | reads <= checked, (inputs, reads, checked)
    print("PASS form: navigate, read, type and click in session main submitted")
    print(f"     name={NAME!r}; every call asked the arbiter first")


def visible(world):
    """The desktop's own Chromium is the one driven: the host frame shows the page."""
    api, surfer, site = world["api"], world["surfer"], world["site"]
    row = chat(
        api,
        surfer,
        f"In browser session main, browser_navigate to {site.url(f'red-{MARKER}')}.",
    )
    assert all(succeeded(call) for call in calls(row, "browser_navigate"))
    red = share(screen(host_frame(surfer)))
    assert red > 0.5, red
    viewer = world["viewer"].desktop(world["address"], world["ca"], surfer)
    seen = share(screen(viewer.frame()))
    viewer.close()
    assert seen > 0.5, seen
    print(f"PASS visible: the host's frame of the guest screen is {red:.0%} page red,")
    print("     and a viewer's frame through the orchestrator shows the same page")


def share(pixels):
    """How much of an RGB screen is the red page."""
    red = sum(tuple(pixels[at : at + 3]) == RED for at in range(0, len(pixels), 3))
    return red / (len(pixels) // 3)


def failures(world):
    """An unreachable page and an unknown element are failed calls, not successes."""
    api, surfer = world["api"], world["surfer"]
    row = chat(
        api, surfer, "In browser session main, browser_navigate to http://127.0.0.1:1/."
    )
    navigate = calls(row, "browser_navigate")[0]["outcome"]
    assert navigate["code"] == 1, navigate
    assert navigate["output"].startswith("Navigation failed: net::ERR_"), navigate
    row = chat(api, surfer, "browser_click element number 99999999 in session main.")
    click = calls(row, "browser_click")[0]["outcome"]
    assert click["code"] == 1 and "failed" in click["output"], click
    row = chat(api, surfer, "browser_read browser session main.")
    assert all(succeeded(call) for call in calls(row, "browser_read"))
    print("PASS failures: an unreachable URL and an unknown element fail with the")
    print("     browser's reason, and the session still works")


def screenshot(world, postgres):
    """A page screenshot's PNG is in the ledger and left out of the view."""
    api, surfer = world["api"], world["surfer"]
    row = chat(api, surfer, "Take a browser_screenshot of browser session main.")
    shot = calls(row, "browser_screenshot")[0]
    assert (
        shot["outcome"]["status"] == "screenshot" and "image" not in shot["outcome"]
    ), shot
    stored = psql(
        postgres,
        "SELECT substr(payload->'outcome'->>'image',1,8) FROM turn_events "
        f"WHERE payload->>'kind'='tool_result' AND payload->>'id'='{shot['id']}'",
    )
    assert stored == ["iVBORw0K"], stored
    print(
        "PASS screenshot: a screenshot outcome whose PNG is in the ledger and left out"
    )
    print("     of the conversation view")


def sessions(world):
    """Task sessions have their own page and cookies; closing one leaves the rest."""
    api, tasks, site = world["api"], world["tasks"], world["site"]
    chat(
        api,
        tasks,
        f"In browser session main, browser_navigate to {site.url('set-home')}.",
    )
    site.saw("set-home")
    row = chat(
        api,
        tasks,
        f"In browser session research, browser_navigate to {site.url('set-trip')}, "
        f"then browser_navigate to {site.url('check-research')}.",
    )
    _, cookies = site.saw("check-research")[-1]
    assert f"trip={MARKER}" in cookies and "home=" not in cookies, cookies
    assert {call["input"]["session"] for call in calls(row, "browser_navigate")} == {
        "research"
    }
    row = chat(api, tasks, "browser_read browser session main.")
    tree = calls(row, "browser_read")[0]["outcome"]["output"]
    assert 'RootWebArea "Set home"' in tree, tree
    chat(
        api,
        tasks,
        f"In browser session main, browser_navigate to {site.url('check-main')}.",
    )
    _, cookies = site.saw("check-main")[-1]
    assert f"home={MARKER}" in cookies and "trip=" not in cookies, cookies
    print("PASS sessions: research kept its own cookie, never saw main's, and its")
    print("     navigation left main's page where it was")

    row = chat(
        api,
        tasks,
        "browser_close browser session research, then browser_read browser session "
        "research.",
    )
    assert calls(row, "browser_close")[0]["outcome"]["output"] == "Closed."
    read = calls(row, "browser_read")[0]["outcome"]
    assert read["output"].startswith("No open browser session has this name"), read
    chat(
        api,
        tasks,
        "In browser session research, browser_navigate to "
        f"{site.url('check-reopened')}.",
    )
    _, cookies = site.saw("check-reopened")[-1]
    assert "trip=" not in cookies, cookies
    row = chat(api, tasks, "browser_read browser session main.")
    assert (
        'RootWebArea "check-main"' in calls(row, "browser_read")[0]["outcome"]["output"]
    )
    print("PASS close: a closed task session is gone, reopening it starts without its")
    print("     cookies, and main still works")

    row = chat(
        api,
        tasks,
        "In one response, call browser_navigate twice in parallel: session alpha to "
        f"{site.url('check-alpha')} and session beta to {site.url('check-beta')}.",
    )
    site.saw("check-alpha"), site.saw("check-beta")
    sessions = {call["input"]["session"] for call in calls(row, "browser_navigate")}
    assert {"alpha", "beta"} <= sessions, sessions
    assert all(succeeded(call) for call in calls(row, "browser_navigate"))
    print(
        "PASS parallel: two task sessions navigated in one turn, each to its own page"
    )


def control(world):
    """While a person holds the desktop, browser input is refused; reads work."""
    api, surfer, site = world["api"], world["surfer"], world["site"]
    controller = world["controller"].desktop(world["address"], world["ca"], surfer)
    controller.frame()
    assert controller.ask({"kind": "take"})["mode"] == "human"
    held = f"held-{MARKER}"
    row = chat(
        api, surfer, f"In browser session main, browser_navigate to {site.url(held)}."
    )
    navigate = calls(row, "browser_navigate")
    assert all(
        call["outcome"]["output"] == "a person has control" for call in navigate
    ), navigate
    checks = mode_checks(surfer, row)
    assert checks and all("error" not in event for event in checks), checks
    row = chat(api, surfer, "browser_read browser session main.")
    assert all(succeeded(call) for call in calls(row, "browser_read"))
    time.sleep(3)
    assert not site.requests(held), site.requests(held)
    assert controller.ask({"kind": "hand_back"})["mode"] == "watch"
    controller.close()
    chat(api, surfer, f"In browser session main, browser_navigate to {site.url(held)}.")
    site.saw(held)
    print("PASS control: with a person in control the agent's navigation was refused,")
    print("     never reached the site, a read worked, and after hand-back it went")


def persistence(world):
    """The main session's profile is on the workspace disk and outlives the sandbox."""
    api, surfer, site = world["api"], world["surfer"], world["site"]
    chat(
        api,
        surfer,
        f"In browser session main, browser_navigate to {site.url('set-kept')}.",
    )
    site.saw("set-kept")
    # Chromium writes cookies to its profile on a 30 s timer, not on each change.
    time.sleep(40)
    old = container(surfer)
    lifecycle(api, surfer, "restart")
    wait_for("a new sandbox", lambda: container(surfer) not in (None, old), 300)
    ready(api, surfer)
    chat(
        api,
        surfer,
        f"In browser session main, browser_navigate to {site.url('check-kept')}.",
    )
    _, cookies = site.saw("check-kept")[-1]
    assert f"kept={MARKER}" in cookies, cookies
    row = chat(
        api,
        surfer,
        "Use the bash tool to run exactly: "
        "ls /var/lib/cubebot/workspace/.config/chromium/Default/Cookies",
    )
    listed = [call for call in row["terminal"] if call["command"].startswith("ls ")]
    assert listed and listed[0]["outcome"].get("code") == 0, row["terminal"]
    print("PASS persistence: a main-session cookie survived a restart of the sandbox,")
    print("     from the profile on the workspace disk")


def scenarios(world, postgres):
    form(world)
    visible(world)
    failures(world)
    screenshot(world, postgres)
    sessions(world)
    control(world)
    persistence(world)


def setup(admin, base, ca, password):
    key = Path(os.environ["OPENAI_API_KEY_FILE"]).read_text().strip()
    provider = command(
        expected_revision=0,
        api_key=key,
        model=MODEL,
        provider="openai",
        context_window=128000,
    )
    admin("/api/provider", provider)
    surfer, tasks = agent(admin, "Surfer"), agent(admin, "Tasks")
    controller = person(admin, base, ca, "surf-controller")
    viewer = person(admin, base, ca, "surf-viewer")
    grant(admin, viewer.id, surfer, "browser_view")
    grant(admin, controller.id, surfer, "browser_view")
    grant(admin, controller.id, surfer, "browser_control")
    return {
        "surfer": surfer,
        "tasks": tasks,
        "controller": controller,
        "viewer": viewer,
    }


def qualify(directory: Path):
    with socket.socket() as probe:
        probe.bind((bridge(), 0))
        address = f"{bridge()}:{probe.getsockname()[1]}"
    environment = configure(directory, address)
    config = directory / "docker.json"
    config.write_text(
        json.dumps(json.loads(config.read_text()) | {"computer_use_enabled": True})
    )
    base, ca = f"https://{address}", directory / "ca/ca.crt"
    admin = Api(base, ca)
    world = {"address": address, "ca": ca, "api": admin}
    log_path = directory / "orchestrator.log"

    def workflow(postgres, env):
        environment["CUBEBOT_DATABASE_URL"] = env["CUBEBOT_TEST_DATABASE_URL"]
        binary = [str(ROOT / "target/debug/cubebot-orchestrator")]
        with open(log_path, "w") as log:
            process = subprocess.Popen(
                binary,
                env=os.environ | environment,
                stdout=log,
                stderr=subprocess.STDOUT,
            )
        try:
            wait_for("orchestrator HTTPS", lambda: live(admin), timeout=30)
            password = secrets.token_urlsafe(24)
            bootstrap(admin, env["CUBEBOT_TEST_DATABASE_URL"], "surf-admin", password)
            world.update(setup(admin, base, ca, password))
            for identity in (world["surfer"], world["tasks"]):
                ready(admin, identity)
            world["site"] = Site()
            scenarios(world, postgres)
        except BaseException:
            print(log_path.read_text()[-20000:])
            for identity in (world.get("surfer"), world.get("tasks")):
                if identity and container(identity):
                    print(logs(identity)[-20000:])
            if os.environ.get("CUBEBOT_BROWSER_HOLD"):
                traceback.print_exc()
                print("HOLDING for inspection", flush=True)
                time.sleep(900)
            raise
        finally:
            for identity in (world.get("surfer"), world.get("tasks")):
                if identity:
                    remove(identity)
            process.terminate()
            process.wait(timeout=30)
        print("PASS the browser tools on real agents")

    with tempfile.TemporaryDirectory(
        prefix="cubebot-postgres-", dir=ROOT / "target"
    ) as data:
        password = secrets.token_hex(24)
        name = f"cubebot-test-{uuid.uuid4().hex}"
        env = os.environ | {"POSTGRES_PASSWORD": password}
        run_database(name, env, password, os.path.abspath(data), workflow)


def main():
    if sys.platform != "linux" or platform.machine() != "x86_64":
        print(f"docker-browser skipped: needs x86_64 Linux, this is {sys.platform}")
        return 0
    if not os.environ.get("OPENAI_API_KEY_FILE"):
        print("docker-browser failed: set OPENAI_API_KEY_FILE", file=sys.stderr)
        return 1
    build()
    with tempfile.TemporaryDirectory() as directory:
        qualify(Path(directory))
    return 0


if __name__ == "__main__":
    sys.exit(main())
