"""One agent through the Docker sandbox backend, without a cluster.

CUBEBOT_SANDBOX_RUNTIME picks the sandbox runtime for this and every docker_*.py
suite: `kvm` (the default where /dev/kvm exists) needs x86_64 Linux with /dev/kvm and
the kernel build tools of tooling/guest; `container` needs only Docker. Both need
OpenSSL and OPENAI_API_KEY_FILE. The orchestrator serves HTTPS on the docker0
address, or on Docker Desktop on loopback, which sandboxes reach as
host.docker.internal, with a CA from `just dev-ca`; a disposable PostgreSQL holds its
state. A real model is asked for one bash tool call; the assertion reads the ledger's
tool call and result, never the model's reply.

`docker_sandbox.py agent NAME` instead creates and wakes an agent against a running
`just docker-orchestrator`, bootstrapping the administrator from
CUBEBOT_BOOTSTRAP_LOGIN/CUBEBOT_BOOTSTRAP_PASSWORD and the provider from
OPENAI_API_KEY_FILE the first time.
"""

import json
import os
import platform
import secrets
import socket
import ssl
import subprocess
import sys
import tempfile
import urllib.error
import urllib.request
import uuid
from pathlib import Path

from cluster_chat_setup import wait_for
from integration import read_env_file, run_database
from smoke import running

ROOT = Path(__file__).resolve().parents[2]
RUNTIME = os.environ.get("CUBEBOT_SANDBOX_RUNTIME") or (
    "kvm" if os.path.exists("/dev/kvm") else "container"
)
# The kvm runtime runs the VMM's image, the container runtime the agent image itself.
IMAGES = {"kvm": "cubebot-sandbox:dev", "container": "cubebot-agent:dev"}
IMAGE = IMAGES.get(RUNTIME, "")
MARKER = "docker-sandbox-ok"
INSTRUCTIONS = (
    "You operate a test sandbox. Run each command you are given with one bash tool "
    "call, exactly as written, then reply with the output. Never rerun a command."
)


class Api:
    def __init__(self, base, ca):
        self.base = base
        self.context = ssl.create_default_context(cafile=str(ca))
        self.token = ""

    def __call__(self, path, body=None, expected=200, method=None):
        headers = {"Authorization": f"Bearer {self.token}"}
        data = None
        if body is not None:
            data = json.dumps(body).encode()
            headers["Content-Type"] = "application/json"
        request = urllib.request.Request(
            self.base + path, data=data, headers=headers, method=method
        )
        try:
            with urllib.request.urlopen(
                request, timeout=10, context=self.context
            ) as response:
                status, payload = response.status, response.read()
        except urllib.error.HTTPError as error:
            status, payload = error.code, error.read()
        assert status == expected, f"{path}: {status} {payload[:500]!r}"
        return json.loads(payload or b"null")


def command(**values):
    return dict(request_id=str(uuid.uuid4()), **values)


def bridge():
    """The host address sandboxes reach: docker0, or on Docker Desktop the loopback it
    forwards host.docker.internal to."""
    if sys.platform == "darwin":
        return "127.0.0.1"
    output = subprocess.run(
        ["ip", "-4", "-o", "addr", "show", "docker0"],
        capture_output=True,
        text=True,
        check=True,
    ).stdout
    return output.split()[3].split("/")[0]


def sign_in(api, database, login, password):
    """Bootstraps the administrator unless one exists, then logs in."""
    admin = ["cargo", "run", "--locked", "-q", "-p", "cubebot-orchestrator"]
    subprocess.run(
        admin + ["--bin", "cubebot-admin", "bootstrap"],
        cwd=ROOT,
        env=os.environ
        | {
            "CUBEBOT_DATABASE_URL": database,
            "CUBEBOT_BOOTSTRAP_LOGIN": login,
            "CUBEBOT_BOOTSTRAP_PASSWORD": password,
        },
        capture_output=True,
        check=False,
    )
    body = {"login": login, "password": password}
    api.token = api("/api/identity/login", body)["token"]


def agent(api, name):
    """Sets the OpenAI provider if none is set, then creates and wakes an agent."""
    if api("/api/provider") is None:
        key = Path(os.environ["OPENAI_API_KEY_FILE"]).read_text().strip()
        provider = command(
            expected_revision=0,
            api_key=key,
            model="gpt-4.1-mini",
            provider="openai",
            context_window=128000,
        )
        api("/api/provider", provider)
    created = command(name=name, instructions=INSTRUCTIONS)
    identity = api("/api/agents", created, expected=201)["id"]
    status = api(f"/api/agents/{identity}/status")
    wake = command(expected_revision=status["revision"], action="wake")
    api(f"/api/agents/{identity}/lifecycle", wake, expected=202)
    return identity


def create(name):
    settings = read_env_file()
    port = settings.get("POSTGRES_PORT", "5432")
    database = (
        f"postgres://cubebot:{settings['POSTGRES_PASSWORD']}@127.0.0.1:{port}/cubebot"
    )
    api = Api(f"https://{bridge()}:8443", ROOT / ".cubebot/dev-ca/ca.crt")
    login = os.environ["CUBEBOT_BOOTSTRAP_LOGIN"]
    sign_in(api, database, login, os.environ["CUBEBOT_BOOTSTRAP_PASSWORD"])
    print(agent(api, name))


def reachable(address):
    """`address` as a sandbox names it."""
    if sys.platform == "darwin":
        return address.replace("127.0.0.1", "host.docker.internal")
    return address


def unsupported(suite):
    """Prints why `suite` cannot run here in RUNTIME and returns its exit status, or
    None."""
    if RUNTIME not in IMAGES:
        print(
            f"{suite} failed: CUBEBOT_SANDBOX_RUNTIME is kvm or container",
            file=sys.stderr,
        )
        return 1
    if RUNTIME == "kvm" and (sys.platform != "linux" or platform.machine() != "x86_64"):
        print(
            f"{suite} skipped: kvm needs x86_64 Linux with /dev/kvm, this host is "
            f"{sys.platform}/{platform.machine()}; "
            "CUBEBOT_SANDBOX_RUNTIME=container runs here"
        )
        return 0
    if RUNTIME == "kvm" and not os.access("/dev/kvm", os.R_OK | os.W_OK):
        print(f"{suite} failed: /dev/kvm is not accessible", file=sys.stderr)
        return 1
    if not os.environ.get("OPENAI_API_KEY_FILE"):
        print(f"{suite} failed: set OPENAI_API_KEY_FILE", file=sys.stderr)
        return 1
    print(f"{suite}: {RUNTIME} sandboxes")
    return None


def build(runtime=RUNTIME):
    subprocess.run(
        ["cargo", "build", "--locked", "-p", "cubebot-orchestrator"],
        check=True,
        cwd=ROOT,
    )
    if runtime == "container":
        subprocess.run(
            ["docker", "build", "--tag", IMAGES[runtime]]
            + ["--file", "apps/agent/Dockerfile", "."],
            check=True,
            cwd=ROOT,
        )
        return
    subprocess.run([sys.executable, ROOT / "tooling/guest/build.py"], check=True)
    subprocess.run(
        ["docker", "build", "--tag", IMAGES[runtime]]
        + ["--build-context", "guest=target/guest"]
        + ["--file", "apps/sandbox/Dockerfile", "."],
        check=True,
        cwd=ROOT,
    )


def configure(directory: Path, address, runtime=RUNTIME):
    """The orchestrator's environment for Docker sandboxes of `runtime`."""
    host = address.rsplit(":", 1)[0]
    names = f"IP:{host}"
    if reachable(host) != host:
        names = f"DNS:{reachable(host)},{names}"
    subprocess.run(
        ["just", "dev-ca", str(directory / "ca"), names], check=True, cwd=ROOT
    )
    key = directory / "credential.key"
    key.write_bytes(secrets.token_bytes(32))
    key.chmod(0o600)
    config = directory / "docker.json"
    config.write_text(
        json.dumps(
            {
                "image": IMAGES[runtime],
                "orchestrator_origin": f"https://{reachable(address)}",
                "runtime": {"kind": runtime},
                "control_ca_file": str(directory / "ca/ca.crt"),
                "terminal_enabled": True,
            }
        )
    )
    return {
        "CUBEBOT_BIND_ADDR": address,
        "CUBEBOT_TLS_CERT_FILE": str(directory / "ca/server.crt"),
        "CUBEBOT_TLS_KEY_FILE": str(directory / "ca/server.key"),
        "CUBEBOT_CREDENTIAL_KEY_FILE": str(key),
        "CUBEBOT_DOCKER_CONFIG": str(config),
        "RUST_LOG": "info",
    }


def tool_calls(postgres, turn):
    query = (
        f"SELECT payload::text FROM turn_events WHERE turn_id = '{turn}' "
        "AND payload->>'kind' IN ('tool_call','tool_result') ORDER BY sequence"
    )
    rows = subprocess.run(
        ["docker", "exec", postgres, "psql", "-U", "cubebot", "-d", "cubebot_test"]
        + ["-Atc", query],
        capture_output=True,
        text=True,
        check=True,
    ).stdout
    calls = {}
    for event in map(json.loads, rows.splitlines()):
        if event["kind"] == "tool_call":
            calls[event["id"]] = {"input": event["input"], "outcome": None}
        else:
            calls[event["id"]]["outcome"] = event["outcome"]
    return list(calls.values())


def bash_turn(api, postgres, identity, marker=MARKER):
    path = f"/api/conversations/{identity}/messages"
    turn = api(path, command(message=f"Run: echo {marker}"), expected=202)["id"]

    def complete():
        rows = api(path)
        return next(
            (row for row in rows if row["id"] == turn and row["status"] == "complete"),
            None,
        )

    reply = wait_for("the bash turn", complete, timeout=180)
    calls = tool_calls(postgres, turn)
    assert len(calls) == 1, (calls, reply)
    assert marker in calls[0]["input"].get("command", ""), calls
    assert marker in json.dumps(calls[0]["outcome"]), calls


def qualify(directory: Path):
    with socket.socket() as probe:
        probe.bind((bridge(), 0))
        address = f"{bridge()}:{probe.getsockname()[1]}"
    environment = configure(directory, address)
    api = Api(f"https://{address}", directory / "ca/ca.crt")

    def workflow(postgres, env):
        database = env["CUBEBOT_TEST_DATABASE_URL"]
        environment["CUBEBOT_DATABASE_URL"] = database
        binary = [str(ROOT / "target/debug/cubebot-orchestrator")]
        with running(binary, ROOT, environment):
            wait_for("orchestrator HTTPS", lambda: live(api), timeout=30)
            sign_in(api, database, "docker-admin", secrets.token_urlsafe(24))
            identity = agent(api, "Docker sandbox")
            try:
                status = f"/api/agents/{identity}/status"
                wait_for(
                    "authenticated runtime",
                    lambda: api(status)["runtime_available"],
                    timeout=300,
                )
                bash_turn(api, postgres, identity)
            finally:
                remove(identity)
        print("PASS one agent booted through Docker and ran one bash tool call")

    with tempfile.TemporaryDirectory(
        prefix="cubebot-postgres-", dir=ROOT / "target"
    ) as data:
        password = secrets.token_hex(24)
        name = f"cubebot-test-{uuid.uuid4().hex}"
        env = os.environ | {"POSTGRES_PASSWORD": password}
        run_database(name, env, password, os.path.abspath(data), workflow)


def live(api):
    try:
        return api("/api/health/live") == {"status": "alive"}
    except (OSError, AssertionError):
        return False


def remove(identity):
    label = f"label=cubebot.dev/agent={identity}"
    containers = subprocess.run(
        ["docker", "ps", "--all", "--quiet", "--filter", label],
        capture_output=True,
        text=True,
        check=False,
    ).stdout.split()
    if containers:
        subprocess.run(["docker", "rm", "--force", *containers], check=False)
    volume = f"cubebot-workspace-{identity}"
    subprocess.run(["docker", "volume", "rm", volume], check=False)


def main():
    if sys.argv[1:2] == ["agent"] and len(sys.argv) == 3:
        return create(sys.argv[2])
    if (refused := unsupported("docker-sandbox")) is not None:
        return refused
    build()
    with tempfile.TemporaryDirectory() as directory:
        qualify(Path(directory))
    return 0


if __name__ == "__main__":
    sys.exit(main())
