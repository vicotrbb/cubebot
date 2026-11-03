"""One agent in the `container` sandbox runtime through Docker, with no provider key.

Runs on every push in CI and on any host with Docker, Docker Desktop on a Mac included.
The orchestrator and a disposable PostgreSQL run as in docker_sandbox.py; the agent
image runs directly with computer use on. A scripted stand-in for the provider listens
inside the agent's own network namespace, at the loopback origin the agent's test seam
(CUBEBOT_TEST_PROVIDER_ORIGIN) accepts, and answers one message with one bash tool call.
The ledger must show that call and its result. Then, through the host channel (the
console's desktop socket, which the orchestrator relays to `cubebot-agent desktop` in
the sandbox), a screenshot must show the wallpaper, a person's key must be refused until
they take control, and must then reach a window the tool user maps on its own display.
This proves the container runtime boots and wires up; it is not agent qualification.
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
)
from guest_boot import photograph
from integration import run_database
from smoke import running

PORT = 18080
KEY_A = 38
STAND_IN = """
import http.server, json, sys

class Provider(http.server.BaseHTTPRequestHandler):
    def do_POST(self):
        body = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
        last = json.dumps(body["input"][-1])
        if sys.argv[2] in last and "function_call_output" not in last:
            arguments = json.dumps({"command": "echo " + sys.argv[2]})
            output = [{"type": "function_call", "call_id": "call-1", "name": "bash",
                       "arguments": arguments}]
        else:
            output = [{"type": "message",
                       "content": [{"type": "output_text", "text": "done"}]}]
        event = {"type": "response.completed",
                 "response": {"status": "completed", "output": output}}
        data = ("data: " + json.dumps(event) + "\\n\\n").encode()
        self.send_response(200)
        self.send_header("Content-Type", "text/event-stream")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

http.server.HTTPServer(("127.0.0.1", int(sys.argv[1])), Provider).serve_forever()
"""


def stand_in(name, marker):
    """Starts the scripted provider in the agent's network namespace, as nobody."""
    subprocess.run(
        ["docker", "exec", "--detach", "--user", "65534:65534", name]
        + ["python3", "-c", STAND_IN, str(PORT), marker],
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
    """guest_desktop.py's key receiver as the tool user on its display, :1."""
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

        def wallpaper():
            return photograph(ask(desktop, {"action": "screenshot"}))

        wait_for("the wallpaper", wallpaper, timeout=60)
        print("PASS the host channel's screenshot shows the wallpaper")
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


def qualify(directory: Path):
    with socket.socket() as probe:
        probe.bind((bridge(), 0))
        address = f"{bridge()}:{probe.getsockname()[1]}"
    environment = configure(directory, address, "container")
    config = directory / "docker.json"
    config.write_text(
        json.dumps(json.loads(config.read_text()) | {"computer_use_enabled": True})
    )
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
                host_channel(address, ca, api.token, identity, name)
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
    with tempfile.TemporaryDirectory() as directory:
        qualify(Path(directory))
    print(f"container-check passed in {time.monotonic() - started:.0f}s")
    return 0


if __name__ == "__main__":
    sys.exit(main())
