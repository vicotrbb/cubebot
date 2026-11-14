"""Backup to MinIO while an agent works, then restore into a fresh installation, for
Compose and for kind.

Compose installs this checkout with deploy/install.sh from a local registry, as
install_check.py does; kind installs the chart as cluster_chat.py does. Both use the
`container` runtime and a scripted model in the agent's own sandbox, so no provider key
is needed. CUBEBOT_BACKUP_CHECK=compose or kind runs one; both run by default. kind
needs what `just cluster-chat` needs except /dev/kvm and a provider key. Not
qualification of real agents, external tokens or scale.
"""

import base64
import json
import os
import secrets
import shutil
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.request
import uuid
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
MINIO = "bitnamilegacy/minio:2025.7.23"
MINIO_DIGEST = "sha256:8935e75fa5d11295c17171e4aa49efe390a1193cd7f12e4d21b92af9ffef09d7"
BUCKET = "cubebot-backups"
MINIO_USER = "backup-check"
MINIO_SECRET = secrets.token_hex(16)
PORT = 18080
MARKER = secrets.token_hex(4)
CONTENT = f"kept-{MARKER}\n"
SHARED = f"shared file {MARKER}\n".encode()
INSTRUCTIONS = "Run exactly the commands you are asked for."

# A model that answers each request with one bash call, then "done".
STAND_IN = r"""
import http.server, json, sys

MARKER = sys.argv[2]
STEPS = {
    "write-" + MARKER: "mkdir -p notes && printf 'kept-" + MARKER
    + r"\n' > notes/keep.txt",
    "work-" + MARKER: "for i in $(seq 1 75); do echo $i >> live.log; sleep 1; done",
}


class Provider(http.server.BaseHTTPRequestHandler):
    def do_POST(self):
        body = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
        items = body["input"]
        done = [i for i in items if i.get("type") == "function_call_output"]
        last = next(i for i in reversed(items) if i.get("role") == "user")
        asked = str(last["content"])
        command = next((c for key, c in STEPS.items() if key in asked), None)
        text = {"type": "output_text", "text": "done"}
        output = [{"type": "message", "content": [text]}]
        if command and not done:
            output = [{"type": "function_call", "call_id": "call-0", "name": "bash",
                       "arguments": json.dumps({"command": command})}]
        frame = {"type": "response.completed",
                 "response": {"status": "completed", "output": output}}
        data = ("data: " + json.dumps(frame) + "\n\n").encode()
        self.send_response(200)
        self.send_header("Content-Type", "text/event-stream")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def log_message(self, *_):
        pass


http.server.HTTPServer(("127.0.0.1", int(sys.argv[1])), Provider).serve_forever()
"""


def wait_for(description, operation, timeout=180, every=1):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        result = operation()
        if result:
            return result
        time.sleep(every)
    raise RuntimeError(f"timed out waiting for {description}")


def run(*args, **options):
    options.setdefault("check", True)
    return subprocess.run(list(args), cwd=ROOT, **options)


class Console:
    """The console API with a bearer session; `call` is JSON, `raw` is bytes."""

    def __init__(self, base, context=None):
        self.base, self.context, self.token = base, context, ""

    def raw(self, method, path, data=None, headers=None, timeout=20):
        headers = dict(headers or {})
        if self.token:
            headers["Authorization"] = f"Bearer {self.token}"
        request = urllib.request.Request(
            self.base + path, data=data, headers=headers, method=method
        )
        try:
            with urllib.request.urlopen(
                request, timeout=timeout, context=self.context
            ) as response:
                return response.status, response.read()
        except urllib.error.HTTPError as error:
            return error.code, error.read()

    def call(self, path, body=None, expected=200, method=None):
        data = None if body is None else json.dumps(body).encode()
        method = method or ("GET" if body is None else "POST")
        status, payload = self.raw(
            method, path, data, {"Content-Type": "application/json"}
        )
        assert status == expected, f"{method} {path}: {status} {payload[:400]!r}"
        return json.loads(payload or b"null")

    def answers(self, path):
        try:
            return self.raw("GET", path, timeout=5)[0] == 200
        except OSError:
            return False

    def sign_in(self, password):
        body = {"login": "admin", "password": password}
        self.token = self.call("/api/identity/login", body)["token"]


def command(**values):
    return dict(request_id=str(uuid.uuid4()), **values)


def restore_state(console):
    try:
        status, payload = console.raw("GET", "/api/restore", timeout=5)
    except OSError:
        return None
    return json.loads(payload) if status == 200 else None


def turn(console, identity, message):
    path = f"/api/conversations/{identity}/messages"
    return console.call(path, command(message=message), expected=202)["id"]


def status_of(console, identity, turn_id):
    rows = console.call(f"/api/conversations/{identity}/messages")
    return next((row["status"] for row in rows if row["id"] == turn_id), None)


def backups(console):
    return console.call("/api/installation/backup")


def finished(console, number):
    view = backups(console)
    done = [b for b in view["backups"] if b["status"] != "running"]
    return (not view["running"]) and len(done) >= number and done[0]


def seed(console, shell):
    """An agent with a workspace file and a shared file; returns the agent."""
    provider = command(
        expected_revision=0,
        api_key="backup-check-key",
        model="backup-check-model",
        provider="openai",
        context_window=128000,
    )
    console.call("/api/provider", provider)
    created = command(name="Backup check", instructions=INSTRUCTIONS)
    identity = console.call("/api/agents", created, expected=201)["id"]
    status = f"/api/agents/{identity}/status"
    wait_for(
        "the agent's runtime",
        lambda: console.call(status)["runtime_available"],
        timeout=420,
    )
    shell.provider(identity)
    written = turn(console, identity, f"write-{MARKER}")
    wait_for(
        "the writing turn",
        lambda: status_of(console, identity, written) == "complete",
        timeout=120,
    )
    assert shell.read(identity, "notes/keep.txt") == CONTENT
    query = f"/api/files?name=shared.txt&request_id={uuid.uuid4()}"
    status, payload = console.raw("POST", query, SHARED)
    assert status == 201, (status, payload[:300])
    return identity


def lines(shell, identity):
    text = shell.read(identity, "live.log", missing="")
    return len(text.split())


def back_up_while_working(console, shell, identity):
    """A backup while a tool call keeps writing; nothing waits for either."""
    working = turn(console, identity, f"work-{MARKER}")
    wait_for("the working turn", lambda: lines(shell, identity) >= 3, timeout=120)
    before = lines(shell, identity)
    console.call("/api/installation/backup/run", {}, expected=202)
    slowest, started = 0.0, time.monotonic()
    while backups(console)["running"]:
        asked = time.monotonic()
        console.call("/api/installation")
        slowest = max(slowest, time.monotonic() - asked)
        assert time.monotonic() - started < 900, "the backup took over 15 minutes"
        time.sleep(1)
    after = lines(shell, identity)
    state = status_of(console, identity, working)
    backup = finished(console, 1)
    assert backup and backup["status"] == "complete", backup
    assert backup["missing"] == [], backup
    assert state in ("running", "complete"), state
    assert after > before, (before, after)
    assert slowest < 3, f"the console took {slowest:.1f} s to answer during the backup"
    print(
        f"PASS backup {backup['id']} verified while a tool call wrote "
        f"{after - before} lines; the console answered within {slowest:.2f} s"
    )
    wait_for(
        "the working turn to finish",
        lambda: status_of(console, identity, working) == "complete",
        timeout=180,
    )
    return backup


def second_backup(console, blobs):
    """Shared files never change, so a second backup sends none again."""
    before = blobs()
    console.call("/api/installation/backup/run", {}, expected=202)
    wait_for("the second backup", lambda: finished(console, 2), timeout=600)
    latest = finished(console, 2)
    assert latest["status"] == "complete", latest
    assert blobs() == before, (before, blobs())
    print("PASS a second backup verified and sent no shared file again")


def key_once(console):
    status, key = console.raw("POST", "/api/installation/backup/key", b"")
    assert status == 200, (status, key[:200])
    assert len(base64.b64decode(key.strip())) == 32
    again, _ = console.raw("POST", "/api/installation/backup/key", b"")
    assert again == 409, again
    downloaded = backups(console)["key"]
    assert downloaded["downloaded_by"] == "admin", downloaded
    return key.decode().strip()


def restored(console, shell, identity, backup, password):
    seen = set()

    def done():
        state = restore_state(console)
        if state:
            seen.add(state["state"])
            assert state["state"] != "failed", state
        return state and state["state"] == "none"

    wait_for("the restore", done, timeout=600)
    assert "restoring" in seen or seen == {"none"}, seen
    wait_for("the restored console", lambda: console.answers("/api/health/ready"))
    console.sign_in(password)
    names = [a["name"] for a in console.call("/api/agents")]
    assert "Backup check" in names, names
    files = console.call("/api/files")
    shared = next(f for f in files if f["name"] == "shared.txt")
    status, data = console.raw("GET", f"/api/files/{shared['id']}/content")
    assert (status, data) == (200, SHARED), (status, data[:100])
    history = {b["id"]: b["status"] for b in backups(console)["backups"]}
    assert history.get(backup["id"]) == "complete", history

    def workspace_back():
        rows = backups(console)["restores"]
        row = next((r for r in rows if r["agent"] == identity), None)
        assert row is None or not row["error"], row
        return row and row["restored_at_unix"]

    wait_for("the agent's workspace", workspace_back, timeout=600)
    assert shell.read(identity, "notes/keep.txt") == CONTENT
    assert lines(shell, identity) >= 3
    print(
        "PASS the fresh installation restored its data, the shared file and the "
        "agent's workspace files"
    )


# Compose


class DockerShell:
    """An agent's sandbox as a Docker container."""

    def provider(self, identity):
        from docker_desktop import container

        name = container(identity)
        run(
            "docker",
            "exec",
            "--detach",
            "--user",
            "65534:65534",
            name,
            "python3",
            "-c",
            STAND_IN,
            str(PORT),
            MARKER,
        )
        probe = f"import socket; socket.create_connection(('127.0.0.1', {PORT}), 1)"
        wait_for(
            "the scripted model",
            lambda: (
                run(
                    "docker",
                    "exec",
                    name,
                    "python3",
                    "-c",
                    probe,
                    check=False,
                    capture_output=True,
                ).returncode
                == 0
            ),
            timeout=30,
        )

    def read(self, identity, path, missing=None):
        from docker_desktop import container

        name = wait_for("the agent's container", lambda: container(identity))
        found = run(
            "docker",
            "exec",
            name,
            "cat",
            f"/var/lib/cubebot/workspace/{path}",
            check=False,
            capture_output=True,
            text=True,
        )
        if found.returncode and missing is not None:
            return missing
        assert found.returncode == 0, found.stderr
        return found.stdout


def compose_project(home):
    checksum = run("cksum", input=str(home), capture_output=True, text=True)
    return f"cubebot-{checksum.stdout.split()[0]}"


def staged_source(directory):
    """The release files install.sh fetches, with the scripted model's address added."""
    source = directory / "source"
    (source / "deploy").mkdir(parents=True)
    shutil.copy(ROOT / "deploy/cubebot", source / "deploy/cubebot")
    for name in ("compose.domain.yaml", "compose.agents.yaml", "compose.control.yaml"):
        shutil.copy(ROOT / name, source / name)
    text = (ROOT / "compose.yaml").read_text()
    anchor = "      RUST_LOG: ${RUST_LOG:-info}\n"
    assert anchor in text, "compose.yaml changed; update backup_check.py"
    origin = f"      CUBEBOT_TEST_PROVIDER_ORIGIN: http://127.0.0.1:{PORT}/\n"
    (source / "compose.yaml").write_text(text.replace(anchor, anchor + origin))
    return source


def minio_objects(container, prefix):
    found = run(
        "docker",
        "exec",
        container,
        "sh",
        "-c",
        f"find /bitnami/minio/data/{BUCKET}/{prefix} -name xl.meta | wc -l",
        capture_output=True,
        text=True,
    )
    return int(found.stdout.strip())


def compose_backup(env, home, minio, endpoint, password):
    import install_check as compose

    compose.install(env)
    settings = {
        "CUBEBOT_BACKUP_ENDPOINT": endpoint,
        "CUBEBOT_BACKUP_BUCKET": BUCKET,
        "CUBEBOT_BACKUP_ACCESS_KEY_ID": MINIO_USER,
        "CUBEBOT_BACKUP_SECRET_ACCESS_KEY": MINIO_SECRET,
        "CUBEBOT_BACKUP_SCHEDULE": "off",
    }
    with (home / ".env").open("a") as dotenv:
        dotenv.writelines(f"{key}={value}\n" for key, value in settings.items())
    run(str(home / "cubebot"), "up", env=env)
    console = Console(f"http://localhost:{env['CUBEBOT_PORT']}")
    wait_for("the console", lambda: console.answers("/api/health/ready"))
    console.sign_in(password)
    view = backups(console)
    assert view["deployment"]["bucket"] == BUCKET, view["deployment"]
    assert view["deployment_source"] == ".env", view["deployment_source"]
    assert view["deployment_secret_access_key"], view
    saved = console.call(
        "/api/installation/backup",
        {"keep_daily": 3, "revision": view["revision"]},
        method="PUT",
    )
    assert saved["settings"]["keep_daily"] == 3, saved["settings"]
    print("PASS .env set the bucket, shown read-only; the console saved retention")
    shell = DockerShell()
    identity = seed(console, shell)
    backup = back_up_while_working(console, shell, identity)
    second_backup(console, lambda: minio_objects(minio, "blobs"))
    key = key_once(console)
    printed = run(
        str(home / "cubebot"), "key", env=env, capture_output=True, text=True
    ).stdout.strip()
    assert printed == key, "cubebot key and the console's download differ"
    print("PASS the console offered the key once; cubebot key prints the same key")
    return identity, backup, key


def compose_restore(env, home, identity, backup, key, password):
    import install_check as compose

    wrong = base64.b64encode(secrets.token_bytes(32)).decode()
    compose.install(env | {"CUBEBOT_RESTORE_KEY": wrong})
    console = Console(f"http://localhost:{env['CUBEBOT_PORT']}")

    def refused():
        state = restore_state(console)
        return state and state["state"] == "failed" and state

    state = wait_for("the wrong key to be refused", refused, timeout=300)
    assert "credential key" in state["error"], state
    assert not console.answers("/api/identity/setup"), "the console opened"
    print("PASS a wrong credential key stopped the restore before it wrote anything")
    run(str(home / "cubebot"), "down", env=env)
    compose.remove_volumes(compose_project(home))
    dotenv = home / ".env"
    kept = [
        line
        for line in dotenv.read_text().splitlines()
        if not line.startswith("CUBEBOT_RESTORE_KEY=")
    ]
    dotenv.write_text("\n".join(kept + [f"CUBEBOT_RESTORE_KEY={key}"]) + "\n")
    clean = {k: v for k, v in env.items() if not k.startswith("CUBEBOT_RESTORE")}
    run(str(home / "cubebot"), "up", env=clean)
    restored(console, DockerShell(), identity, backup, password)


def compose_check(directory):
    import install_check as compose
    from docker_sandbox import bridge, remove

    registry_port, minio_port, port = (compose.free_port() for _ in range(3))
    registry = f"localhost:{registry_port}"
    stamp = f"cubebot-backup-check-{MARKER}"
    host = bridge()
    endpoint = (
        f"http://host.docker.internal:{minio_port}"
        if sys.platform == "darwin"
        else f"http://{host}:{minio_port}"
    )
    source = staged_source(directory)
    homes = (directory / "a", directory / "b")
    base = os.environ | {
        "CUBEBOT_VERSION": compose.TAG,
        "CUBEBOT_REGISTRY": registry,
        "CUBEBOT_SOURCE": f"file://{source}",
        "CUBEBOT_RUNTIME": "container",
        "CUBEBOT_PORT": str(port),
    }
    original = base | {
        "CUBEBOT_HOME": str(homes[0]),
        "CUBEBOT_RESTORE": "no",
        "CUBEBOT_ADMIN": "now",
        "CUBEBOT_BOOTSTRAP_LOGIN": "admin",
        "CUBEBOT_BOOTSTRAP_PASSWORD": compose.PASSWORD,
    }
    fresh = base | {
        "CUBEBOT_HOME": str(homes[1]),
        "CUBEBOT_RESTORE": "yes",
        "CUBEBOT_RESTORE_ENDPOINT": endpoint,
        "CUBEBOT_RESTORE_REGION": "us-east-1",
        "CUBEBOT_RESTORE_BUCKET": BUCKET,
        "CUBEBOT_RESTORE_ACCESS_KEY_ID": MINIO_USER,
        "CUBEBOT_RESTORE_SECRET_ACCESS_KEY": MINIO_SECRET,
        "CUBEBOT_RESTORE_BACKUP": "newest",
    }
    agents = []
    try:
        compose.docker(
            "run",
            "--detach",
            "--name",
            f"{stamp}-registry",
            "--publish",
            f"127.0.0.1:{registry_port}:5000",
            "registry:2",
        )
        compose.docker(
            "run",
            "--detach",
            "--name",
            f"{stamp}-minio",
            "--publish",
            f"{host}:{minio_port}:9000",
            "--env",
            f"MINIO_ROOT_USER={MINIO_USER}",
            "--env",
            f"MINIO_ROOT_PASSWORD={MINIO_SECRET}",
            "--env",
            f"MINIO_DEFAULT_BUCKETS={BUCKET}",
            f"{MINIO}@{MINIO_DIGEST}",
        )
        compose.publish(registry)
        identity, backup, key = compose_backup(
            original, homes[0], f"{stamp}-minio", endpoint, compose.PASSWORD
        )
        agents.append(identity)
        # A lost machine: its containers, volumes and agents all go.
        run(str(homes[0] / "cubebot"), "uninstall", "--yes", env=original)
        compose_restore(fresh, homes[1], identity, backup, key, compose.PASSWORD)
    except BaseException:
        for home, env in zip(homes, (original, fresh), strict=True):
            if (home / "cubebot").exists():
                run(str(home / "cubebot"), "logs", "--tail", "80", env=env, check=False)
        raise
    finally:
        for home, env in zip(homes, (original, fresh), strict=True):
            if (home / "cubebot").exists():
                run(str(home / "cubebot"), "uninstall", "--yes", env=env, check=False)
            compose.remove_volumes(compose_project(home))
        for identity in agents:
            remove(identity)
        for suffix in ("registry", "minio"):
            compose.docker(
                "rm", "--force", "--volumes", f"{stamp}-{suffix}", check=False
            )
        for image in ("orchestrator", "web", "agent"):
            compose.docker(
                "rmi", f"{registry}/cubebot-{image}:{compose.TAG}", check=False
            )
    print(
        "PASS Compose: backed up while an agent worked, restored into a fresh install"
    )


# kind


class KubeShell:
    """An agent's sandbox as a pod in the fixture's namespace."""

    def __init__(self, fixture):
        self.f = fixture

    def pod(self, identity):
        def running():
            items = json.loads(
                self.f.k(
                    "get", "pods", "-l", f"cubebot.dev/agent={identity}", "-o", "json"
                )
            )["items"]
            ready = [p for p in items if p["status"].get("phase") == "Running"]
            return ready and ready[0]["metadata"]["name"]

        return wait_for("the agent's pod", running, timeout=300)

    def provider(self, identity):
        import shlex

        pod = self.pod(identity)
        script = shlex.quote(STAND_IN)
        start = (
            f"setsid python3 -c {script} {PORT} {MARKER} </dev/null >/dev/null 2>&1 &"
        )
        self.f.k("exec", pod, "--", "sh", "-c", start)
        probe = f"import socket; socket.create_connection(('127.0.0.1', {PORT}), 1)"

        def listening():
            try:
                self.f.k("exec", pod, "--", "python3", "-c", probe)
                return True
            except RuntimeError:
                return False

        wait_for("the scripted model", listening, timeout=30)

    def read(self, identity, path, missing=None):
        pod = self.pod(identity)
        try:
            return (
                self.f.k("exec", pod, "--", "cat", f"/var/lib/cubebot/workspace/{path}")
                + "\n"
            )
        except RuntimeError:
            if missing is not None:
                return missing
            raise


def kind_minio(f):
    import cluster_admission as admission

    # The node pulls it: kind load imports every platform, and only one was pulled here.
    reference = f"{MINIO}@{MINIO_DIGEST}"
    labels = {"app": "minio"}
    manifest = {
        "apiVersion": "v1",
        "kind": "List",
        "items": [
            {"apiVersion": "v1", "kind": "Namespace", "metadata": {"name": "minio"}},
            {
                "apiVersion": "v1",
                "kind": "Pod",
                "metadata": {"name": "minio", "namespace": "minio", "labels": labels},
                "spec": {
                    "containers": [
                        {
                            "name": "minio",
                            "image": reference,
                            "env": [
                                {"name": "MINIO_ROOT_USER", "value": MINIO_USER},
                                {"name": "MINIO_ROOT_PASSWORD", "value": MINIO_SECRET},
                                {"name": "MINIO_DEFAULT_BUCKETS", "value": BUCKET},
                            ],
                            "ports": [{"containerPort": 9000}],
                            "readinessProbe": {
                                "httpGet": {"path": "/minio/health/live", "port": 9000}
                            },
                        }
                    ]
                },
            },
            {
                "apiVersion": "v1",
                "kind": "Service",
                "metadata": {"name": "minio", "namespace": "minio"},
                "spec": {"selector": labels, "ports": [{"port": 9000}]},
            },
        ],
    }
    admission.kubectl(f.config, "apply", "-f", "-", input=json.dumps(manifest))
    admission.kubectl(
        f.config,
        "-n",
        "minio",
        "wait",
        "--for=condition=Ready",
        "pod/minio",
        "--timeout=180s",
        timeout=200,
    )
    return "http://minio.minio:9000"


def bucket_secret(f, name):
    f.k(
        "create",
        "secret",
        "generic",
        name,
        f"--from-literal=access-key-id={MINIO_USER}",
        f"--from-literal=secret-access-key={MINIO_SECRET}",
    )


def kind_backup(f, endpoint):
    from cluster_chat_setup import NAMESPACE, ORIGIN, available, helm

    bucket_secret(f, "backup-bucket")
    helm(
        f,
        "upgrade",
        "cubebot",
        "deploy/chart",
        "--namespace",
        NAMESPACE,
        "--reuse-values",
        "--set",
        f"backup.endpoint={endpoint},backup.bucket={BUCKET}",
        "--set",
        "backup.existingSecret=backup-bucket,backup.schedule=off",
    )
    origin = f"CUBEBOT_TEST_PROVIDER_ORIGIN=http://127.0.0.1:{PORT}/"
    f.k("set", "env", "deployment/orchestrator", origin)
    f.k("rollout", "status", "deployment/orchestrator", "--timeout=300s", timeout=320)
    wait_for("the console", lambda: available(f), timeout=300)
    console = Console(ORIGIN, f.context)
    console.sign_in(f.password)
    view = backups(console)
    assert view["deployment_source"] == "Helm values", view["deployment_source"]
    assert view["deployment"]["bucket"] == BUCKET, view["deployment"]
    print("PASS Helm values set the bucket, shown read-only with their source")
    shell = KubeShell(f)
    identity = seed(console, shell)
    backup = back_up_while_working(console, shell, identity)

    def blobs():
        listing = f.k(
            "-n",
            "minio",
            "exec",
            "minio",
            "--",
            "sh",
            "-c",
            f"find /bitnami/minio/data/{BUCKET}/blobs -name xl.meta | wc -l",
        )
        return int(listing.strip())

    second_backup(console, blobs)
    return identity, backup, key_once(console)


def kind_restore(f, identity, backup, key, endpoint):
    import cluster_admission as admission
    from cluster_chat_setup import NAMESPACE, ORIGIN, helm

    # A lost cluster: the release and everything in its namespace go.
    admission.run(
        "helm",
        "--kubeconfig",
        str(f.config),
        "uninstall",
        "cubebot",
        "--namespace",
        NAMESPACE,
        "--wait",
        timeout=330,
    )
    # Cubebot's retention finalizers outlive the orchestrator that would release them.
    for kind in ("pods", "persistentvolumeclaims", "configmaps"):
        for name in f.k("get", kind, "-o", "name").split():
            f.k("patch", name, "--type=merge", "-p", '{"metadata":{"finalizers":null}}')
    admission.kubectl(f.config, "delete", "namespace", NAMESPACE, "--wait", timeout=600)
    admission.namespaces(
        f.config, f.name, ((NAMESPACE, "baseline"),), runtime_account=False
    )
    private = f.artifacts / "private"
    f.k(
        "create",
        "secret",
        "tls",
        "tls",
        f"--cert={private / 'tls.crt'}",
        f"--key={private / 'tls.key'}",
    )
    key_file = private / "credential.key"
    key_file.write_bytes(base64.b64decode(key))
    key_file.chmod(0o600)
    f.k(
        "create",
        "secret",
        "generic",
        "cubebot-credentials",
        f"--from-file=key={key_file}",
    )
    key_file.unlink()
    bucket_secret(f, "restore-bucket")
    values = json.loads((f.artifacts / "values.json").read_text())
    values["credentials"] = {"existingSecret": "cubebot-credentials"}
    values["restore"] = {
        "enabled": True,
        "endpoint": endpoint,
        "bucket": BUCKET,
        "existingSecret": "restore-bucket",
    }
    path = f.artifacts / "values-restore.json"
    path.write_text(json.dumps(values, indent=2))
    helm(
        f,
        "install",
        "cubebot",
        "deploy/chart",
        "--namespace",
        NAMESPACE,
        "--values",
        str(path),
    )
    console = Console(ORIGIN, f.context)
    restored(console, KubeShell(f), identity, backup, f.password)


def kind_qualify(f):
    endpoint = kind_minio(f)
    identity, backup, key = kind_backup(f, endpoint)
    kind_restore(f, identity, backup, key, endpoint)
    print("PASS kind: backed up while an agent worked, restored into a fresh install")


def kind_check():
    os.environ["CUBEBOT_SANDBOX_RUNTIME"] = "container"
    evidence = Path(tempfile.mkdtemp(prefix="cubebot-backup-kind-"))
    os.environ.setdefault("CUBEBOT_QUALIFICATION_DIRECTORY", str(evidence))
    import cluster_chat

    # The scripted agent runs bash, which only a terminal-enabled sandbox has.
    cluster_chat.main(
        prefix="cubebot-backup-", qualify=kind_qualify, terminalEnabled=True
    )


def main():
    which = os.environ.get("CUBEBOT_BACKUP_CHECK", "compose,kind").split(",")
    started = time.monotonic()
    (ROOT / "target").mkdir(exist_ok=True)
    with tempfile.TemporaryDirectory(
        prefix="cubebot-backup-", dir=ROOT / "target"
    ) as d:
        if "compose" in which:
            compose_check(Path(d))
        if "kind" in which:
            kind_check()
    print(f"backup-check passed in {time.monotonic() - started:.0f} s")


if __name__ == "__main__":
    sys.exit(main())
