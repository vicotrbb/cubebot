"""One agent in the `container` sandbox runtime through Docker, with no provider key.

Runs in CI and on any Docker host. Proves the container runtime boots and wires up; it
is not agent qualification.
"""

import json
import os
import secrets
import socket
import subprocess
import sys
import tempfile
import time
import uuid
from pathlib import Path

from cluster_chat_setup import wait_for
from docker_desktop import Desktop, container
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
    remove,
    sign_in,
    tool_calls,
)
from guest_boot import rgb
from integration import run_database
from smoke import running

PORT = 18080
KEY_A = 38
STAND_IN = r"""
import http.server, json, re, sys

MARKER, SCRIPT = sys.argv[2], sys.argv[3]


# A model reads a tool's outcome as JSON, so the reply inside it is escaped.
def id_in(output):
    return re.search(r'id\\":\\"([0-9a-f-]{36})', output).group(1)


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


def step(request, outputs):
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
    assert {"coding", "design", "write-a-poem"} <= set(library), library
    installed = [s["name"] for s in api(f"/api/agents/{identity}/skills")]
    assert sorted(installed) == ["coding", "design", "write-a-poem"], installed
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


def qualify(directory: Path, network):
    with socket.socket() as probe:
        probe.bind((bridge(), 0))
        address = f"{bridge()}:{probe.getsockname()[1]}"
    environment = configure(directory, address, "container")
    config = directory / "docker.json"
    settings = {"computer_use_enabled": True, "network": network}
    config.write_text(json.dumps(json.loads(config.read_text()) | settings))
    environment["CUBEBOT_STORAGE_DIRECTORY"] = str(directory / "storage")
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
            sign_in(api, database, "container-admin", secrets.token_urlsafe(24))
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
                openrouter_turn(api, postgres, identity, marker)
                print("PASS one turn through the OpenRouter adapter")
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
    network = f"cubebot-check-{uuid.uuid4().hex[:12]}"
    subprocess.run(
        ["docker", "network", "create", "--opt"]
        + ["com.docker.network.bridge.enable_icc=false", network],
        check=True,
        stdout=subprocess.DEVNULL,
    )
    try:
        with tempfile.TemporaryDirectory() as directory:
            qualify(Path(directory), network)
    finally:
        subprocess.run(["docker", "network", "rm", network], check=False)
    print(f"container-check passed in {time.monotonic() - started:.0f}s")
    return 0


if __name__ == "__main__":
    sys.exit(main())
