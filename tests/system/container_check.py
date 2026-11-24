"""One agent in the `container` sandbox runtime through Docker, with no provider key.

Runs in CI and on any Docker host. Proves the container runtime boots and wires up; it
is not agent qualification.
"""

import http.server
import json
import os
import secrets
import socket
import subprocess
import sys
import tempfile
import threading
import time
import urllib.error
import urllib.parse
import urllib.request
import uuid
from pathlib import Path

from cluster_chat_setup import wait_for
from docker_desktop import CONSOLE, Desktop, container, surroundings
from docker_sandbox import (
    IMAGES,
    INSTRUCTIONS,
    ROOT,
    Api,
    agent,
    bash_turn,
    bridge,
    build,
    command,
    configure,
    live,
    reachable,
    remove,
    sign_in,
    tool_calls,
)
from guest_boot import rgb
from integration import run_database
from smoke import running

PORT = 18080
# The skills apps/orchestrator/skills ships, installed on every agent.
SHIPPED = ("build-app", "coding", "create-agent", "create-room-or-team", "design")
KEY_A = 38
STAND_IN = r"""
import http.server, json, re, sys

MARKER, SCRIPT = sys.argv[2], sys.argv[3]


def id_in(output):
    return re.search(r'id\\?":\\?"([0-9a-f-]{36})', output).group(1)


# A person's whole request as one model would make it, one step per reply: write a
# file, hand it to the person the turn says it serves, then use the knowledge base.
def scripted(system, outputs):
    login = re.search(r"the login (\S+?)\.", system).group(1)
    steps = [
        lambda: ("bash", {"command": "echo verse > poem.txt"}),
        lambda: ("files_put",
                 {"path": "poem.txt", "name": "poem.txt", "parent": None}),
        lambda: ("files_share", {"file": id_in(outputs[1]), "kind": "user",
                                 "name": login, "permission": "read"}),
        lambda: ("kb_create", {"kind": "page", "title": "Check " + MARKER,
                               "body": "first words", "parent": None}),
        lambda: ("kb_search", {"query": MARKER}),
        lambda: ("kb_edit", {"page": id_in(outputs[3]), "old_text": "first",
                             "new_text": "agent"}),
        lambda: ("skill_create", {"name": "write-a-poem",
                                  "description": "Write and share a short poem.",
                                  "instructions": "Write it, then share it.",
                                  "files": []}),
        lambda: ("send_attachment", {"path": "poem.txt", "name": "poem.txt"}),
    ]
    return steps[len(outputs)]() if len(outputs) < len(steps) else None


# A request through connected accounts: find the tools, call one, then look for the
# credential from the tool user's shell, where it must not be.
def connected(request, outputs):
    steps = [
        lambda: ("find_tools", {"query": "status-page get"}),
        lambda: ("status-page__get", {"path": "/echo", "query": {}}),
        lambda: ("bash", {"command": "env; ps -eo args"}),
    ]
    if "with-github" in request:
        steps += [
            lambda: ("find_tools", {"query": "github issue"}),
            lambda: ("github__get_issue", {"repo": "rust-lang/rust", "number": 1}),
        ]
    if "with-feed" in request:
        steps += [
            lambda: ("find_tools", {"query": "news feed"}),
            lambda: ("news__read_feed", {"feed": 1, "limit": 3}),
        ]
    if "with-nextcloud" in request:
        upload = {"path": "cubebot-check.txt", "workspace_file": "upload-check.txt"}
        steps += [
            lambda: ("bash", {"command": "printf cubebot-upload > upload-check.txt"}),
            lambda: ("find_tools", {"query": "cloud upload download"}),
            lambda: ("cloud__upload", upload | {"overwrite": True}),
            lambda: ("cloud__download", {"path": "cubebot-check.txt"}),
        ]
    if "with-sql" in request:
        steps += [
            lambda: ("find_tools", {"query": "database sql query"}),
            lambda: ("data__query", {"sql": "SELECT current_user AS who"}),
        ]
    return steps[len(outputs)]() if len(outputs) < len(steps) else None


APP = '''<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Status</title>
<link rel="stylesheet" href="./_cubebot/kit.css">
<script src="./_cubebot/kit.js"></script>
</head>
<body>
<main class="page">
<header class="page__head"><h1>Status</h1></header>
<div class="page__body">
<div class="kit-grid">
<section class="kit-card">
<h2>Status page</h2>
<p id="result">Loading</p>
</section>
</div>
</div>
</main>
<script>
var shown = document.getElementById('result')
cubebot.call('status-page__get', { path: '/echo', query: {} }).then(
  function (text) { shown.textContent = 'Live: ' + text },
  function (error) { shown.textContent = 'Failed: ' + error.message })
</script>
</body>
</html>'''


def built(outputs):
    app = {"description": "The status page, live.", "icon": "\U0001F4DF",
           "folder": "apps/status", "command": None, "port": None}
    steps = [
        lambda: ("bash", {"command": "mkdir -p apps/status && cat > "
                          "apps/status/index.html <<'EOF'\n" + APP + "\nEOF"}),
        lambda: ("app_register", app | {"slug": "status", "name": "Status",
                                        "tools": ["status-page__get"]}),
        lambda: ("app_register", app | {"slug": "notes", "name": "Notes",
                                        "tools": []}),
    ]
    return steps[len(outputs)]() if len(outputs) < len(steps) else None


# Two commands the watchdog must flag: the runtime's private state, then a miner.
def watched(outputs):
    steps = [
        lambda: ("bash", {"command": "cat /run/cubebot/display 2>&1; true"}),
        lambda: ("bash", {"command": "./xmrig --version 2>&1; true"}),
    ]
    return steps[len(outputs)]() if len(outputs) < len(steps) else None


# What the sandbox sensor must see: a program run from /tmp, then a private
# address that never answers, held open across a few of its samples.
def sensed(outputs):
    command = (
        "cp /bin/sleep /tmp/watch-check && /tmp/watch-check 8; "
        "timeout 9 bash -c '</dev/tcp/10.255.255.1/80'; true"
    )
    return ("bash", {"command": command}) if not outputs else None


def step(request, outputs):
    if "sensor-" + MARKER in request:
        return sensed(outputs)
    if "watchdog-" + MARKER in request:
        return watched(outputs)
    if "apps-" + MARKER in request:
        return built(outputs)
    if "connections-" + MARKER in request:
        return connected(request, outputs)
    if SCRIPT in request:
        return scripted(request, outputs)
    if MARKER in request and not outputs:
        return ("bash", {"command": "echo " + MARKER})
    return None


class Provider(http.server.BaseHTTPRequestHandler):
    def do_POST(self):
        body = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
        answer = self.chat if "completions" in self.path else self.openai
        frames = answer(body)
        data = "".join("data: " + frame + "\n\n" for frame in frames).encode()
        self.send_response(200)
        self.send_header("Content-Type", "text/event-stream")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def openai(self, body):
        items = body["input"]
        done = [i["output"] for i in items if i.get("type") == "function_call_output"]
        last = next(i for i in reversed(items) if i.get("role") == "user")
        call = step(items[0]["content"] + last["content"], done)
        text = {"type": "output_text", "text": "done"}
        output = [{"type": "message", "content": [text]}]
        if call:
            output = [{"type": "function_call", "call_id": "call-" + str(len(done)),
                       "name": call[0], "arguments": json.dumps(call[1])}]
        return [json.dumps({"type": "response.completed",
                            "response": {"status": "completed", "output": output}})]

    # OpenRouter's stream as documented: a tool call in pieces, then usage.
    def chat(self, body):
        messages = body["messages"]
        done = [m["content"] for m in messages if m["role"] == "tool"]
        last = next(m for m in reversed(messages) if m["role"] == "user")
        call = step(last["content"], done)
        chunk = lambda delta, finish=None: json.dumps(
            {"choices": [{"index": 0, "delta": delta, "finish_reason": finish}]})
        if not call:
            frames = [chunk({"content": "done"}), chunk({}, "stop")]
        else:
            arguments = json.dumps(call[1])
            frames = [
                chunk({"tool_calls": [{"index": 0, "id": "call-0", "function":
                                       {"name": call[0], "arguments": ""}}]}),
                chunk({"tool_calls": [{"index": 0, "function":
                                       {"arguments": arguments[:4]}}]}),
                chunk({"tool_calls": [{"index": 0, "function":
                                       {"arguments": arguments[4:]}}]}),
                chunk({}, "tool_calls"),
            ]
        usage = {"choices": [], "usage": {"prompt_tokens": 10, "completion_tokens": 2}}
        return frames + [json.dumps(usage), "[DONE]"]


http.server.HTTPServer(("127.0.0.1", int(sys.argv[1])), Provider).serve_forever()
"""


def scripted_request(marker):
    return f"Write a poem and share it with me, {marker}"


def finished(api, identity, turn):
    rows = api(f"/api/conversations/{identity}/messages")
    return next(
        (row for row in rows if row["id"] == turn and row["status"] == "complete"),
        None,
    )


def shared_and_written(api, postgres, identity, marker):
    path = f"/api/conversations/{identity}/messages"
    message = command(message=scripted_request(marker))
    turn = api(path, message, expected=202)["id"]
    try:
        wait_for(
            "the scripted turn", lambda: finished(api, identity, turn), timeout=120
        )
    except RuntimeError:
        print(json.dumps(tool_calls(postgres, turn), indent=1)[:4000])
        raise
    calls = tool_calls(postgres, turn)
    assert len(calls) == 8, calls
    for call in calls:
        assert call["outcome"]["code"] == 0, call
    shared = [entry["name"] for entry in api("/api/files/shared")]
    assert shared == ["poem.txt"], shared
    pages = [p for p in api("/api/kb/pages") if marker in p["title"]]
    assert len(pages) == 1, pages
    page = api(f"/api/kb/pages/{pages[0]['id']}")
    assert page["body"] == "agent words", page
    assert page["updated_by"]["agent"] == identity, page
    # A skill the agent wrote is a real library entry it has installed, not a file.
    library = {skill["name"]: skill for skill in api("/api/skills")}
    assert set(SHIPPED) | {"write-a-poem"} <= set(library), library
    installed = [s["name"] for s in api(f"/api/agents/{identity}/skills")]
    assert sorted(installed) == sorted([*SHIPPED, "write-a-poem"]), installed
    reply = finished(api, identity, turn)
    assert [a["name"][:4] for a in reply["attachments"]] == ["poem"], reply
    assert reply["attachments"][0]["size"] == len("verse\n"), reply
    definition = api(f"/api/agents/{identity}")
    update = command(
        expected_revision=definition["revision"],
        name=definition["name"],
        instructions=definition["instructions"],
        idle_policy=definition["idle_policy"],
        idle_delay_seconds=definition["idle_delay_seconds"],
        dreaming_model=definition["dreaming_model"],
        knowledge_base=False,
    )
    api(f"/api/agents/{identity}/definition", update)
    assert not api(f"/api/agents/{identity}")["knowledge_base"]


def watchdog_turn(api, identity, marker):
    path = f"/api/conversations/{identity}/messages"
    message = command(message=f"Tidy the workspace, watchdog-{marker}")
    turn = api(path, message, expected=202)["id"]
    wait_for("the watchdog turn", lambda: finished(api, identity, turn), timeout=120)

    # The installation's own findings, such as the providers this check set, aside.
    def flagged():
        found = [f for f in api("/api/watchdog")["findings"] if f["agent"] == identity]
        return found if len(found) >= 2 else None

    findings = wait_for("the watchdog's findings", flagged, timeout=60)
    # Every earlier turn in this check was ordinary work, so nothing else may be open.
    rules = sorted(finding["rule"] for finding in findings)
    assert rules == ["credential_reach", "miner"], findings
    for finding in findings:
        expected = (identity, turn, "high")
        assert (finding["agent"], finding["turn"], finding["severity"]) == expected
    checks = {check["key"]: check["ok"] for check in api("/api/watchdog")["checks"]}
    assert checks["watchdog"] and "sandbox" in checks, checks
    told = [item for item in api("/api/activity") if item["kind"] == "watchdog"]
    assert len(told) == 2, told
    miner = next(finding for finding in findings if finding["rule"] == "miner")
    dismiss = f"/api/watchdog/findings/{miner['id']}/dismiss"
    api(dismiss, {"mute": True}, expected=204, method="PUT")
    after = api("/api/watchdog")
    mine = [f["rule"] for f in after["findings"] if f["agent"] == identity]
    assert mine == ["credential_reach"], after
    muted = [(mute["rule"], mute["agent"]) for mute in after["mutes"]]
    assert muted == [("miner", identity)], after


def sensor_turn(api, identity, marker):
    path = f"/api/conversations/{identity}/messages"
    message = command(message=f"Check the sandbox, sensor-{marker}")
    turn = api(path, message, expected=202)["id"]
    wait_for("the sensor turn", lambda: finished(api, identity, turn), timeout=120)

    def sensed():
        findings = api("/api/watchdog")["findings"]
        found = {f["rule"]: f for f in findings if f["agent"] == identity}
        wanted = {"hidden_executable", "blocked_private"}
        return found if wanted <= set(found) else None

    found = wait_for("the sensor's findings", sensed, timeout=60)
    hidden = found["hidden_executable"]
    assert hidden["agent"] == identity and "/tmp/watch-check" in hidden["summary"], (
        hidden
    )
    blocked = found["blocked_private"]
    assert "10.255.255.1:80" in blocked["summary"], blocked
    unexpected = set(found) - {
        "credential_reach",
        "hidden_executable",
        "blocked_private",
    }
    assert not unexpected, found


def openrouter_turn(api, postgres, identity, marker):
    provider = command(
        expected_revision=1,
        api_key="scripted-provider-key",
        model="vendor/scripted-model:free",
        provider="openrouter",
        context_window=128000,
    )
    api("/api/provider", provider)
    bash_turn(api, postgres, identity, marker)
    assert [p["provider"] for p in api("/api/provider/saved")] == ["openai"]


class Echo(http.server.BaseHTTPRequestHandler):
    """A service an HTTP API connection reaches; it answers with the header sent."""

    seen = []

    def do_GET(self):
        Echo.seen.append(self.headers.get("Authorization", ""))
        body = json.dumps({"authorization": self.headers.get("Authorization")})
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body.encode())

    def log_message(self, *_):
        pass


def connections_turn(api, postgres, identity, marker, database):
    """A connection's tools run in the agent with a credential bash never sees."""
    echo = http.server.HTTPServer((bridge(), 0), Echo)
    threading.Thread(target=echo.serve_forever, daemon=True).start()
    secret = "echo-" + secrets.token_hex(12)
    origin = f"http://{bridge()}:{echo.server_port}/"
    made = api(
        "/api/connections",
        {
            "connector": "http-api",
            "slug": "status-page",
            "auth": "api_key",
            "config": {"base_url": origin, "allowed_paths": ["/echo"]},
            "secrets": {"token": secret},
        },
    )
    assert made["status"] == "connected", made
    # The orchestrator checked it from the host; the agent names the host differently.
    container_origin = reachable(origin)
    if container_origin != origin:
        moved = json.dumps(container_origin)
        update = (
            f"UPDATE connections SET config = config || '{{\"base_url\": {moved}}}'"
        )
        subprocess.run(
            [
                "docker",
                "exec",
                postgres,
                "psql",
                "-U",
                "cubebot",
                "-d",
                "cubebot_test",
                "-c",
                update,
            ],
            check=True,
            stdout=subprocess.DEVNULL,
        )
    extras = wave_two_connections(api, postgres)
    github = os.environ.get("CUBEBOT_CHECK_GITHUB_TOKEN")
    if github:
        made = api(
            "/api/connections",
            {
                "connector": "github",
                "slug": "github",
                "auth": "api_key",
                "secrets": {"token": github},
            },
        )
        assert made["status"] == "connected", made
    request = (
        f"Check the status page, connections-{marker}"
        + (" with-github" if github else "")
        + "".join(f" with-{name}" for name in extras)
    )
    path = f"/api/conversations/{identity}/messages"
    turn = api(path, command(message=request), expected=202)["id"]
    wait_for("the connections turn", lambda: finished(api, identity, turn), timeout=180)
    calls = tool_calls(postgres, turn)
    found, called, shell = calls[0], calls[1], calls[2]
    assert "status-page__get" in json.dumps(found["outcome"]), found
    assert called["outcome"]["code"] == 0, called
    assert "[credential]" in json.dumps(called["outcome"]), called
    # The orchestrator's check when it was saved, then the agent's own call.
    assert Echo.seen == [f"Bearer {secret}"] * 2, Echo.seen
    journal = subprocess.run(
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
            "SELECT string_agg(payload::text, '') FROM turn_events "
            f"WHERE turn_id = '{turn}'",
        ],
        capture_output=True,
        text=True,
        check=True,
    ).stdout
    assert secret not in journal and secret not in json.dumps(shell), (
        "a credential leaked"
    )
    if github:
        issue = calls[4]
        assert issue["outcome"]["code"] == 0 and github not in journal, issue
        print("PASS an agent read a real GitHub issue through its connection")
    outcome = {call["name"]: call["outcome"] for call in calls}
    if "feed" in extras:
        read = outcome["news__read_feed"]
        assert read["code"] == 0 and "title" in json.dumps(read), read
        print("PASS an agent read a real feed through a connection with no credential")
    if "nextcloud" in extras:
        uploaded = outcome["cloud__upload"]
        downloaded = outcome["cloud__download"]
        assert uploaded["code"] == 0, uploaded
        assert "cubebot-upload" in json.dumps(downloaded), downloaded
        assert extras["nextcloud"] not in journal, "a credential leaked"
        print("PASS an agent uploaded a workspace file to Nextcloud and read it back")
    if "sql" in extras:
        user, password = extras["sql"]
        queried = outcome["data__query"]
        assert queried["code"] == 0 and user in json.dumps(queried), queried
        assert password not in journal, "a credential leaked"
        print("PASS an agent queried PostgreSQL over its own port from the sandbox")
    return echo


POLICY = (
    "sandbox allow-scripts allow-forms allow-popups allow-downloads; "
    "default-src 'self'; "
    "base-uri 'none'; frame-ancestors 'self'; connect-src 'self'; "
    "img-src 'self' data: blob:; media-src 'self' blob:; font-src 'self' data:; "
    "style-src 'self' 'unsafe-inline'; script-src 'self' 'unsafe-inline' "
    "'wasm-unsafe-eval'"
)


def view_policy(api, prefix):
    host = urllib.parse.urlsplit(api.base).netloc
    view = prefix.rstrip("/") + "/"
    sources = " ".join(
        f"{scheme}://{host}{view}" for scheme in ("http", "https", "ws", "wss")
    )
    return POLICY.replace("connect-src 'self'", f"connect-src 'self' {sources}")


def relayed(api, path, method="GET", body=None, headers=None):
    data = json.dumps(body).encode() if body is not None else None
    request = urllib.request.Request(
        api.base + path, data=data, headers=headers or {}, method=method
    )
    if data is not None:
        request.add_header("Content-Type", "application/json")
    try:
        with urllib.request.urlopen(
            request, timeout=30, context=api.context
        ) as response:
            return response.status, response.headers, response.read()
    except urllib.error.HTTPError as error:
        return error.code, error.headers, error.read()


def app_state(api, slug, state):
    apps = {app["slug"]: app for app in api("/api/apps")}
    return apps.get(slug) if apps.get(slug, {}).get("state") == state else None


def apps_turn(api, postgres, identity, marker, console):
    path = f"/api/conversations/{identity}/messages"
    message = command(message=f"Build me a status page app, apps-{marker}")
    turn = api(path, message, expected=202)["id"]
    wait_for("the apps turn", lambda: finished(api, identity, turn), timeout=180)
    calls = tool_calls(postgres, turn)
    assert [call["outcome"]["code"] for call in calls] == [0, 0, 0], calls
    app = wait_for(
        "the app running", lambda: app_state(api, "status", "running"), timeout=60
    )
    assert app["agent_id"] == identity, app
    assert app["connections"] == [{"slug": "status-page", "connector": "http-api"}], app
    print("PASS the agent wrote an app with bash and registered it; it runs")
    prefix = api("/api/apps/status/views", method="POST")["path"]
    assert prefix.startswith("/api/apps/status/v/"), prefix
    status, headers, body = relayed(api, prefix)
    assert status == 200 and b"_cubebot/kit.js" in body, (status, body[:200])
    assert headers.get_all("Content-Security-Policy") == [view_policy(api, prefix)], (
        headers
    )
    assert headers["Access-Control-Allow-Origin"] == "*", headers
    assert headers.get("Set-Cookie") is None, headers
    status, headers, body = relayed(api, prefix + "_cubebot/kit.css")
    assert status == 200 and b"--ground" in body, (status, body[:200])
    assert headers["Content-Type"].startswith("text/css"), headers
    call = {"tool": "status-page__get", "input": {"path": "/echo", "query": {}}}
    status, _, body = relayed(api, prefix + "_cubebot/call", "POST", call)
    answer = json.loads(body)
    assert status == 200 and answer["ok"] is True, answer
    assert "authorization" in answer["text"], answer
    assert "echo-" not in answer["text"], "a credential leaked into the app"
    other = {"tool": "github__get_issue", "input": {}}
    status, _, body = relayed(api, prefix + "_cubebot/call", "POST", other)
    assert status == 403 and json.loads(body)["ok"] is False, (status, body)
    asked = {"Access-Control-Request-Method": "POST", "Origin": "null"}
    asked["Access-Control-Request-Headers"] = "content-type"
    status, headers, _ = relayed(api, prefix + "_cubebot/call", "OPTIONS", None, asked)
    assert status == 204, status
    assert headers["Access-Control-Allow-Headers"] == "content-type", headers
    for escape in ("%2e%2e/%2e%2e/etc/passwd", "..%2f..%2f.cubebot-apps"):
        status, _, body = relayed(api, prefix + escape)
        assert status == 404 and b"root:" not in body, (escape, status, body[:200])
    token = prefix.rstrip("/").rsplit("/", 1)[1]
    status, _, _ = relayed(api, f"/api/apps/notes/v/{token}/")
    assert status == 404, status
    notes = api("/api/apps/notes/views", method="POST")["path"]
    assert relayed(api, notes)[0] == 200
    status, _, _ = relayed(api, notes + "_cubebot/call", "POST", call)
    assert status == 403, status
    print("PASS the relay serves the app with its policy and the kit, calls only the")
    print("     declared tool, answers preflights, and keeps paths and tokens in place")
    api("/api/apps/status/state", {"desired": "stopped"})
    wait_for("the app stopped", lambda: app_state(api, "status", "stopped"), timeout=30)
    assert relayed(api, prefix)[0] == 503
    api("/api/apps/status/state", {"desired": "running"})
    wait_for("the app running", lambda: app_state(api, "status", "running"), timeout=60)
    assert relayed(api, prefix)[0] == 200
    print("PASS Stop and Start from the API reached the agent's app")
    if console:
        console()


def apps_console(address, ca, password):
    evidence = Path(os.environ.get("CUBEBOT_APPS_EVIDENCE", "/tmp/cubebot-apps"))
    evidence.mkdir(parents=True, exist_ok=True)
    names = surroundings({"evidence": evidence}, address, ca)
    try:
        wait_for("the console", lambda: live(Api(CONSOLE, ca)), timeout=60)
        environment = os.environ | {
            "CUBEBOT_CONSOLE": CONSOLE,
            "CUBEBOT_CONSOLE_LOGIN": "container-admin",
            "CUBEBOT_CONSOLE_PASSWORD": password,
            "CUBEBOT_EVIDENCE_DIRECTORY": str(evidence),
        }
        script = Path(__file__).with_name("apps_browser.mjs")
        result = subprocess.run(
            ["node", script],
            env=environment,
            capture_output=True,
            text=True,
            timeout=600,
        )
        print(result.stdout + result.stderr)
        assert result.returncode == 0, "the apps console check failed"
    finally:
        subprocess.run(["docker", "rm", "-f", *names], check=False, capture_output=True)
    print(
        "PASS console: the catalog and the framed app, wide and phone, light and dark;"
    )
    print(f"     {evidence}")


def wave_two_connections(api, postgres):
    """Optional connections to real services, reachable from this host.

    CUBEBOT_CHECK_FEED is a feed address; CUBEBOT_CHECK_NEXTCLOUD is
    `<address> <user> <app password>`; CUBEBOT_CHECK_SQL is a PostgreSQL server with
    TLS as `<host> <port> <database> <user> <password>`.
    """
    extras = {}
    feed = os.environ.get("CUBEBOT_CHECK_FEED")
    if feed:
        made = api(
            "/api/connections",
            {
                "connector": "rss",
                "slug": "news",
                "auth": "public",
                "config": {"feeds": [feed]},
            },
        )
        assert made["status"] == "connected", made
        extras["feed"] = feed
    nextcloud = os.environ.get("CUBEBOT_CHECK_NEXTCLOUD")
    if nextcloud:
        url, user, password = nextcloud.split()
        made = api(
            "/api/connections",
            {
                "connector": "nextcloud",
                "slug": "cloud",
                "auth": "password",
                "config": {"url": url, "username": user},
                "secrets": {"password": password},
            },
        )
        assert made["status"] == "connected", made
        moved = json.dumps(reachable(url))
        update = (
            f"UPDATE connections SET config = config || '{{\"url\": {moved}}}' "
            "WHERE slug = 'cloud'"
        )
        subprocess.run(
            [
                "docker",
                "exec",
                postgres,
                "psql",
                "-U",
                "cubebot",
                "-d",
                "cubebot_test",
                "-c",
                update,
            ],
            check=True,
            stdout=subprocess.DEVNULL,
        )
        extras["nextcloud"] = password
    sql = os.environ.get("CUBEBOT_CHECK_SQL")
    if sql:
        host, port, database, user, password = sql.split()
        config = {"engine": "postgres", "host": host, "port": int(port)}
        made = api(
            "/api/connections",
            {
                "connector": "sql",
                "slug": "data",
                "auth": "password",
                "config": config
                | {"database": database, "username": user, "tls": "require"},
                "secrets": {"password": password},
            },
        )
        assert made["status"] == "connected", made
        moved = json.dumps(reachable(host))
        update = (
            f"UPDATE connections SET config = config || '{{\"host\": {moved}}}' "
            "WHERE slug = 'data'"
        )
        subprocess.run(
            [
                "docker",
                "exec",
                postgres,
                "psql",
                "-U",
                "cubebot",
                "-d",
                "cubebot_test",
                "-c",
                update,
            ],
            check=True,
            stdout=subprocess.DEVNULL,
        )
        extras["sql"] = (user, password)
    return extras


def stand_in(name, marker):
    subprocess.run(
        ["docker", "exec", "--detach", "--user", "65534:65534", name]
        + ["python3", "-c", STAND_IN, str(PORT), marker, scripted_request(marker)],
        check=True,
    )

    def listening():
        probe = f"import socket; socket.create_connection(('127.0.0.1', {PORT}), 1)"
        found = subprocess.run(
            ["docker", "exec", name, "python3", "-c", probe], capture_output=True
        )
        return found.returncode == 0

    wait_for("the scripted provider", listening, timeout=30)


def receiver(name):
    source = (Path(__file__).with_name("guest_desktop.py")).read_text()
    source = source.replace('if __name__ == "__main__":\n    main()', "receive_key()")
    process = subprocess.Popen(
        ["docker", "exec", "--user", "10001:10001", "--env", "DISPLAY=:1", name]
        + ["python3", "-u", "-c", source],
        stdout=subprocess.PIPE,
        text=True,
    )
    line = process.stdout.readline().strip()
    assert line == "Guest desktop ready", line
    return process


def ask(desktop, action):
    return desktop.ask({"kind": "input", "action": action})


def host_channel(address, ca, token, identity, name):
    desktop = Desktop(address, ca, identity, {"Authorization": f"Bearer {token}"})
    try:
        desktop.frame()

        # Chromium opens over the wallpaper; a drawn desktop has far more than bare X.
        def drawn():
            pixels = rgb(ask(desktop, {"action": "screenshot"}).get("png", ""))
            return len({pixels[at : at + 3] for at in range(0, len(pixels), 3)}) > 64

        wait_for("the desktop", drawn, timeout=60)
        print("PASS the host channel's screenshot shows the desktop")
        keys = receiver(name)
        key = {"action": "key", "keys": ["a"]}
        refused = ask(desktop, key)
        assert refused.get("error") == "this person does not have control", refused
        taken = desktop.ask({"kind": "take"})
        assert taken["mode"] == "human" and "error" not in taken, taken
        typed = ask(desktop, key)
        assert "error" not in typed, typed
        received = keys.stdout.readline().strip()
        assert received == f"Guest key {KEY_A} pressed", received
        keys.wait(timeout=10)
        print("PASS a person's key was refused until they took control, then reached")
        print("     the tool user's window")
    finally:
        desktop.close()


def isolated(name, network):
    joined = subprocess.run(
        ["docker", "inspect", "--format", "{{json .NetworkSettings.Networks}}", name],
        capture_output=True,
        text=True,
        check=True,
    ).stdout
    assert list(json.loads(joined)) == [network], joined
    print("PASS the sandbox is on its network only")
    daemon = subprocess.run(
        ["docker", "info", "--format", "{{.OperatingSystem}}"],
        capture_output=True,
        text=True,
        check=True,
    ).stdout.strip()
    # Docker installs its rule there, but the VM's bridge traffic does not meet it.
    if daemon == "OrbStack":
        print("SKIP inter-container traffic: OrbStack does not enforce it")
        return
    peer = f"{network}-peer"
    subprocess.run(
        ["docker", "run", "--detach", "--rm", "--name", peer, "--network", network]
        + ["--entrypoint", "python3", IMAGES["container"]]
        + ["-m", "http.server", "9000"],
        check=True,
        stdout=subprocess.DEVNULL,
    )
    try:
        probe = f"import socket; socket.create_connection(('{peer}', 9000), 3)"
        reached = subprocess.run(
            ["docker", "exec", name, "python3", "-c", probe],
            capture_output=True,
            text=True,
        )
        assert "timed out" in reached.stderr, reached
        print("PASS a neighbour on the sandbox network was unreachable")
    finally:
        subprocess.run(
            ["docker", "rm", "--force", peer], check=False, stdout=subprocess.DEVNULL
        )


def qualify(directory: Path, network, console):
    with socket.socket() as probe:
        probe.bind((bridge(), 0))
        address = f"{bridge()}:{probe.getsockname()[1]}"
    environment = configure(directory, address, "container")
    config = directory / "docker.json"
    settings = {"computer_use_enabled": True, "network": network}
    config.write_text(json.dumps(json.loads(config.read_text()) | settings))
    environment["CUBEBOT_STORAGE_DIRECTORY"] = str(directory / "storage")
    environment["CUBEBOT_PUBLIC_ORIGIN"] = CONSOLE
    # Forwarded to each sandbox; the stand-in listens there.
    environment["CUBEBOT_TEST_PROVIDER_ORIGIN"] = f"http://127.0.0.1:{PORT}/"
    ca = directory / "ca/ca.crt"
    api = Api(f"https://{address}", ca)
    marker = f"container-check-{secrets.token_hex(4)}"

    def workflow(postgres, env):
        database = env["CUBEBOT_TEST_DATABASE_URL"]
        environment["CUBEBOT_DATABASE_URL"] = database
        binary = [str(ROOT / "target/debug/cubebot-orchestrator")]
        with running(binary, ROOT, environment):
            wait_for("orchestrator HTTPS", lambda: live(api), timeout=30)
            password = secrets.token_urlsafe(24)
            sign_in(api, database, "container-admin", password)
            watch = console and (lambda: console(address, ca, password))
            installation = api("/api/installation")
            assert installation["sandbox_runtime"] == "container", installation
            provider = command(
                expected_revision=0,
                api_key="scripted-provider-key",
                model="scripted-model",
                provider="openai",
                context_window=128000,
            )
            api("/api/provider", provider)
            identity = agent(api, "Container check", INSTRUCTIONS)
            try:
                status = f"/api/agents/{identity}/status"
                wait_for(
                    "authenticated runtime",
                    lambda: api(status)["runtime_available"],
                    timeout=300,
                )
                name = container(identity)
                stand_in(name, marker)
                bash_turn(api, postgres, identity, marker)
                print("PASS one agent booted in a container and ran one bash tool call")
                isolated(name, network)
                host_channel(address, ca, api.token, identity, name)
                shared_and_written(api, postgres, identity, marker)
                print(
                    "PASS an agent shared a file by login and wrote the knowledge base"
                )
                echo = connections_turn(api, postgres, identity, marker, database)
                print(
                    "PASS an agent found and called a connection tool with a credential"
                )
                print("     that the model, the journal and the tool user never saw")
                apps_turn(api, postgres, identity, marker, watch)
                echo.shutdown()
                openrouter_turn(api, postgres, identity, marker)
                print("PASS one turn through the OpenRouter adapter")
                watchdog_turn(api, identity, marker)
                print("PASS the watchdog flagged exactly the two dangerous commands,")
                print("     after every earlier turn's ordinary work raised nothing")
                sensor_turn(api, identity, marker)
                print("PASS the sandbox sensor saw a program run from /tmp and a")
                print("     private address left unanswered, and nothing else")
            except BaseException:
                if name := container(identity):
                    subprocess.run(["docker", "logs", "--tail", "80", name])
                raise
            finally:
                remove(identity)

    with tempfile.TemporaryDirectory(
        prefix="cubebot-postgres-", dir=ROOT / "target"
    ) as data:
        password = secrets.token_hex(24)
        name = f"cubebot-test-{uuid.uuid4().hex}"
        env = os.environ | {"POSTGRES_PASSWORD": password}
        run_database(name, env, password, os.path.abspath(data), workflow)


def main():
    started = time.monotonic()
    build("container")
    watched = bool(os.environ.get("CUBEBOT_PLAYWRIGHT_MODULE"))
    if watched:
        web = ["docker", "build", "-q", "-t", "cubebot-web:local"]
        subprocess.run(web + ["-f", "apps/web/Dockerfile", "."], check=True, cwd=ROOT)
    else:
        print(
            "SKIP apps console: set CUBEBOT_PLAYWRIGHT_MODULE to check it in Chromium"
        )
    network = f"cubebot-check-{uuid.uuid4().hex[:12]}"
    subprocess.run(
        ["docker", "network", "create", "--opt"]
        + ["com.docker.network.bridge.enable_icc=false", network],
        check=True,
        stdout=subprocess.DEVNULL,
    )
    try:
        with tempfile.TemporaryDirectory() as directory:
            qualify(Path(directory), network, apps_console if watched else None)
    finally:
        subprocess.run(["docker", "network", "rm", network], check=False)
    print(f"container-check passed in {time.monotonic() - started:.0f}s")
    return 0


if __name__ == "__main__":
    sys.exit(main())
