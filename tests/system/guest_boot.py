"""Acceptance for the kvm backend (ADR 0084): x86_64 Linux, /dev/kvm, Docker and sudo.

A network namespace with a veth pair stands in for an agent pod, shaped as Calico
gives one: a /32 address behind a link-local gateway that the host answers by proxy
ARP. In it the sandbox boots the guest twice on one workspace disk. Each boot must
reach a stand-in orchestrator at the pod's address over TLS with the forwarded
bootstrap token, report liveness over vsock, and on SIGTERM exit with the agent's
status 0, leaving the workspace filesystem clean.
The second boot must reopen the journal lock file the first one created. Before both,
the sandbox must refuse to start in the host's own network namespace. The stand-in
only records the bootstrap request, so this is not agent qualification.
"""

import http.server
import json
import os
import platform
import queue
import signal
import ssl
import subprocess
import sys
import tempfile
import threading
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
GUEST = ROOT / "target/guest"
NAMESPACE = "cubebot-guest-test"
HOST, POD, GATEWAY = "10.231.0.1", "10.231.0.2", "169.254.1.1"
PORT = 8443
TOKEN = "cb_guest_acceptance"
LOCK = "/runtime/tools/lock"
DEADLINE = 60.0
SHUTDOWN = 20.0


def main() -> int:
    if sys.platform != "linux" or platform.machine() != "x86_64":
        print(
            f"guest-boot skipped: the kvm backend needs x86_64 Linux with /dev/kvm, "
            f"this host is {sys.platform}/{platform.machine()}"
        )
        return 0
    # On a Linux host a missing /dev/kvm is a failure, so CI cannot pass by skipping.
    if not os.access("/dev/kvm", os.R_OK | os.W_OK):
        print(
            "guest-boot failed: /dev/kvm is missing or not accessible", file=sys.stderr
        )
        return 1
    subprocess.run([sys.executable, ROOT / "tooling/guest/build.py"], check=True)
    subprocess.run(
        ["cargo", "build", "--locked", "-p", "cubebot-sandbox"], check=True, cwd=ROOT
    )
    pod()
    try:
        with tempfile.TemporaryDirectory() as directory:
            directory = Path(directory)
            return acceptance(directory, orchestrator(directory))
    finally:
        sudo("ip", "netns", "del", NAMESPACE)


def acceptance(directory: Path, requests: queue.Queue) -> int:
    (directory / "token").write_text(TOKEN)
    workspace = directory / "workspace.img"
    failure = host_refusal(directory, workspace)
    if failure:
        print(f"guest-boot failed: {failure}")
        return 1
    inode = None
    for boot in (1, 2):
        failure = run_guest(directory, workspace, requests)
        if failure:
            print(f"guest-boot failed on boot {boot}: {failure}")
            return 1
        if subprocess.run(["e2fsck", "-fn", workspace], capture_output=True).returncode:
            print(f"guest-boot failed: the workspace is not clean after boot {boot}")
            return 1
        stat = subprocess.run(
            ["debugfs", "-R", f"stat {LOCK}", workspace], capture_output=True, text=True
        ).stdout
        if "Inode:" not in stat:
            print(f"guest-boot failed: boot {boot} left no {LOCK} on the workspace")
            return 1
        current = stat.split("Inode:")[1].split()[0]
        if inode not in (None, current):
            print(f"guest-boot failed: boot 2 replaced {LOCK} instead of reopening it")
            return 1
        inode = current
        print(f"guest-boot: boot {boot} passed")
    return 0


def host_refusal(directory: Path, workspace: Path):
    """In the host's network namespace `run` refuses before touching an interface."""
    result = subprocess.run(
        ["sudo", "env"]
        + [
            f"{name}={value}"
            for name, value in environment(directory, workspace).items()
        ]
        + [ROOT / "target/debug/cubebot-sandbox", "run", "--backend", "kvm"],
        capture_output=True,
        text=True,
        timeout=DEADLINE,
    )
    if result.returncode == 0 or "own network namespace" not in result.stderr:
        return f"the sandbox did not refuse the host network namespace: {result.stderr}"
    if (
        subprocess.run(["ip", "link", "show", "tap0"], capture_output=True).returncode
        == 0
    ):
        return "the refused sandbox left a tap device on the host"
    return None


def run_guest(directory: Path, workspace: Path, requests: queue.Queue):
    """Returns why a boot failed, or None."""
    sandbox = subprocess.Popen(
        ["sudo", "ip", "netns", "exec", NAMESPACE, "env"]
        + [
            f"{name}={value}"
            for name, value in environment(directory, workspace).items()
        ]
        + [ROOT / "target/debug/cubebot-sandbox", "run", "--backend", "kvm"],
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
    )
    try:
        return check(sandbox, Console(sandbox.stdout), requests)
    finally:
        if sandbox.poll() is None:
            subprocess.run(["sudo", "pkill", "-KILL", "-x", "cubebot-sandbox"])
        sandbox.wait()


def environment(directory: Path, workspace: Path):
    return {
        "CUBEBOT_GUEST_KERNEL": GUEST / "bzImage",
        "CUBEBOT_GUEST_ROOT": GUEST / "root.ext4",
        "CUBEBOT_GUEST_WORKSPACE": workspace,
        "CUBEBOT_ORCHESTRATOR_ORIGIN": f"https://{HOST}:{PORT}/",
        "CUBEBOT_BOOTSTRAP_TOKEN_FILE": directory / "token",
        "CUBEBOT_CONTROL_CA_FILE": directory / "ca.pem",
    }


class Console:
    """Echoes and keeps sandbox output, so the guest never blocks on a full pipe."""

    def __init__(self, stream):
        self.output = b""
        self.closed = threading.Event()
        threading.Thread(target=self.read, args=(stream,), daemon=True).start()

    def read(self, stream):
        while chunk := os.read(stream.fileno(), 65536):
            sys.stdout.buffer.write(chunk)
            sys.stdout.flush()
            self.output += chunk
        self.closed.set()


def check(sandbox: subprocess.Popen, console: Console, requests: queue.Queue):
    deadline = time.monotonic() + DEADLINE
    if not until(lambda: b"Guest agent alive" in console.output, deadline):
        return "no liveness report over vsock"
    try:
        source, token = requests.get(timeout=max(deadline - time.monotonic(), 0))
    except queue.Empty:
        return "the agent did not reach the orchestrator"
    if (source, token) != (POD, TOKEN):
        return f"bootstrap came from {source} with an unexpected token"
    # Bootstrap retries queue more requests; each boot checks only its own first one.
    while not requests.empty():
        requests.get()
    sandbox.send_signal(signal.SIGTERM)
    if not console.closed.wait(SHUTDOWN) or sandbox.wait(timeout=5) != 0:
        return f"the VM did not exit 0 within {SHUTDOWN}s of SIGTERM"
    for line, failure in [
        (b"Runtime startup failed", "the agent could not start, or open its journal"),
        (b"Guest workspace unmount failed", "the guest did not unmount its workspace"),
    ]:
        if line in console.output:
            return failure
    return None


def until(condition, deadline: float) -> bool:
    while not condition() and time.monotonic() < deadline:
        time.sleep(0.1)
    return condition()


def pod():
    """A namespace whose eth0 reaches the host at HOST, as Calico gives an agent pod."""
    sudo("ip", "netns", "add", NAMESPACE)
    sudo(
        "ip",
        "link",
        "add",
        "cubebot-guest",
        "type",
        "veth",
        "peer",
        "name",
        "eth0",
        "netns",
        NAMESPACE,
    )
    sudo("ip", "addr", "add", f"{HOST}/32", "dev", "cubebot-guest")
    sudo("ip", "link", "set", "cubebot-guest", "up")
    sudo("ip", "route", "add", f"{POD}/32", "dev", "cubebot-guest")
    sudo("sysctl", "-q", "net.ipv4.conf.cubebot-guest.proxy_arp=1")
    inside = ["ip", "-n", NAMESPACE]
    sudo(*inside, "addr", "add", f"{POD}/32", "dev", "eth0")
    sudo(*inside, "link", "set", "eth0", "up")
    sudo(*inside, "route", "add", GATEWAY, "dev", "eth0", "scope", "link")
    sudo(*inside, "route", "add", "default", "via", GATEWAY, "dev", "eth0")


def orchestrator(directory: Path) -> queue.Queue:
    """A TLS listener on HOST that records each bootstrap's source address and token."""
    certificates(directory)
    requests = queue.Queue()

    class Bootstrap(http.server.BaseHTTPRequestHandler):
        def do_POST(self):
            body = self.rfile.read(int(self.headers.get("Content-Length", 0)))
            if self.path == "/api/runtime/bootstrap":
                requests.put((self.client_address[0], json.loads(body).get("token")))
            self.send_response(503)
            self.end_headers()

        def log_message(self, *_):
            pass

    server = http.server.ThreadingHTTPServer((HOST, PORT), Bootstrap)
    context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    context.load_cert_chain(directory / "server.pem", directory / "server.key")
    server.socket = context.wrap_socket(server.socket, server_side=True)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    return requests


def certificates(directory: Path):
    """A private CA and a certificate for HOST: rustls refuses a self-signed leaf."""
    extensions = directory / "server.ext"
    extensions.write_text(
        f"subjectAltName=IP:{HOST}\nbasicConstraints=CA:FALSE\nextendedKeyUsage=serverAuth\n"
    )
    key = ["-newkey", "ec", "-pkeyopt", "ec_paramgen_curve:prime256v1", "-nodes"]
    openssl(
        "req",
        "-x509",
        *key,
        "-keyout",
        "ca.key",
        "-out",
        "ca.pem",
        "-days",
        "1",
        "-subj",
        "/CN=cubebot guest test",
        cwd=directory,
    )
    openssl(
        "req",
        *key,
        "-keyout",
        "server.key",
        "-out",
        "server.csr",
        "-subj",
        f"/CN={HOST}",
        cwd=directory,
    )
    openssl(
        "x509",
        "-req",
        "-in",
        "server.csr",
        "-CA",
        "ca.pem",
        "-CAkey",
        "ca.key",
        "-CAcreateserial",
        "-out",
        "server.pem",
        "-days",
        "1",
        "-extfile",
        extensions,
        cwd=directory,
    )


def openssl(*arguments, cwd: Path):
    subprocess.run(["openssl", *arguments], check=True, capture_output=True, cwd=cwd)


def sudo(*command):
    subprocess.run(["sudo", *command], check=True)


if __name__ == "__main__":
    sys.exit(main())
