"""Acceptance for the kvm backend (ADR 0084): x86_64 Linux, /dev/kvm, Docker and sudo.

A network namespace with a veth pair stands in for an agent pod, shaped as Calico
gives one: a /32 address behind a link-local gateway that the host answers by proxy
ARP. In it the sandbox boots the guest twice on one workspace disk. Each boot must
reach a stand-in orchestrator at the pod's address over TLS with the forwarded
bootstrap token, report liveness over vsock, and on SIGTERM exit with the agent's
status 0, leaving the workspace filesystem clean.
The second boot must reopen the journal lock file the first one created, and runs with
computer use on: a person takes control and closes the maximized Chromium until the host
channel's screenshot shows the wallpaper. Before both,
the sandbox must refuse to start in the host's own network namespace. A third boot
replaces the agent with guest_desktop.py as init (ADR 0085): through the host
channel (`cubebot-sandbox desktop`), a screenshot must be the pattern it drew through a
DRM dumb buffer, a person's key must be refused until that person takes control and then
reach its evdev stream, and the guest's own key request must then be refused as agent
input. The stand-in only records the bootstrap request, so this is not agent
qualification.
"""

import base64
import http.server
import json
import os
import platform
import queue
import signal
import ssl
import struct
import subprocess
import sys
import tempfile
import threading
import time
import zlib
from pathlib import Path

import guest_desktop

ROOT = Path(__file__).resolve().parents[2]
GUEST = ROOT / "target/guest"
NAMESPACE = "cubebot-guest-test"
HOST, POD, GATEWAY = "10.231.0.1", "10.231.0.2", "169.254.1.1"
PORT = 8443
TOKEN = "cb_guest_acceptance"
LOCK = "/runtime/tools/lock"
INIT = "/usr/local/bin/cubebot-agent"
DEADLINE = 60.0
SHUTDOWN = 20.0
# Set either to boot a guest of that size instead of the sandbox's 2 vCPUs and 2 GiB.
SIZES = {
    name: os.environ[name]
    for name in ("CUBEBOT_GUEST_VCPUS", "CUBEBOT_GUEST_MEMORY_MIB")
    if name in os.environ
}
VCPUS = int(SIZES.get("CUBEBOT_GUEST_VCPUS", 2))
PERSON = "00000000-0000-0000-0000-00000000000a"
COMPUTER = {"CUBEBOT_COMPUTER_ENABLED": "1", "CUBEBOT_TOOL_USER": "10001:10001"}


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
    for boot, computer in ((1, {}), (2, COMPUTER)):
        failure = run_guest(directory, workspace, requests, computer)
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
    failure = desktop(directory, workspace)
    if failure:
        print(f"guest-boot failed on the desktop boot: {failure}")
        return 1
    print("guest-boot: desktop boot passed")
    return 0


def host_refusal(directory: Path, workspace: Path):
    """In the host's network namespace `run` refuses before touching an interface."""
    result = subprocess.run(
        ["sudo", "env"]
        + [
            f"{name}={value}"
            for name, value in environment(directory, workspace).items()
        ]
        + [ROOT / "target/debug/cubebot-sandbox", "run"],
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


def run_guest(directory: Path, workspace: Path, requests: queue.Queue, computer: dict):
    """Returns why a boot failed, or None."""
    sandbox = launch(environment(directory, workspace) | computer)
    try:
        console = Console(sandbox.stdout)
        failure = computer and drawn(directory, console)
        return failure or check(sandbox, console, requests)
    finally:
        end(sandbox)


def drawn(directory: Path, console: "Console"):
    """Returns why the desktop never showed the wallpaper, or None."""
    deadline = time.monotonic() + DEADLINE
    if not until(lambda: b"Guest desktop started" in console.output, deadline):
        return "the guest desktop did not start"
    control = connect(directory)
    screenshot = {"input": person({"action": "screenshot"})}
    close = {"input": person({"action": "key", "keys": ["ctrl", "w"]})}

    shown = False

    # Chromium starts maximized over the wallpaper, so close it until the wallpaper shows.
    # Xorg's root, xterm and Chromium have a few hundred colors; the photograph thousands.
    def wallpaper() -> bool:
        nonlocal shown
        shown = shown or colors(ask(control, screenshot)) > 4096
        if not shown:
            ask(control, close)
        return shown

    try:
        if ask(control, {"mode": {"mode": "human", "account": PERSON}}).get("error"):
            return "the person could not take control of the desktop"
        if not until(wallpaper, time.monotonic() + DEADLINE):
            return "the desktop never showed the wallpaper"
        return None
    finally:
        control.stdin.close()
        control.wait()


def colors(screen: dict) -> int:
    pixels = rgb(screen.get("png", ""))
    return len({pixels[at : at + 3] for at in range(0, len(pixels), 3)})


def desktop(directory: Path, workspace: Path):
    """Boots a root image copy with guest_desktop.py as init; returns why it failed."""
    root = directory / "desktop.ext4"
    subprocess.run(["cp", "--reflink=auto", GUEST / "root.ext4", root], check=True)
    script = Path(__file__).with_name("guest_desktop.py")
    commands = directory / "desktop.debugfs"
    commands.write_text(f"rm {INIT}\nwrite {script} {INIT}\nsif {INIT} mode 0100755\n")
    subprocess.run(
        ["debugfs", "-w", "-f", commands, root], check=True, capture_output=True
    )
    sandbox = launch(environment(directory, workspace) | {"CUBEBOT_GUEST_ROOT": root})
    try:
        console = Console(sandbox.stdout)
        deadline = time.monotonic() + DEADLINE
        if not until(lambda: b"Guest desktop drawn" in console.output, deadline):
            return "the guest did not draw through DRM"
        screen = host(directory, {"input": person({"action": "screenshot"})})
        if rgb(screen.get("png", "")) != expected_rgb():
            return "the host channel's screenshot is not the pattern the guest drew"
        key = {"input": person({"action": "key", "keys": ["a"]})}
        if host(directory, key).get("error") != "this person does not have control":
            return "a person's key was not refused before they took control"
        # The arbiter hands control back when the connection that took it closes.
        control = connect(directory)
        try:
            ask(control, {"mode": {"mode": "human", "account": PERSON}})
            if ask(control, key).get("error"):
                return "the person holding control could not type"
            if not until(lambda: b"Guest key 30 pressed" in console.output, deadline):
                return "the A key the person typed did not reach the guest's evdev"
            if not until(lambda: b"Guest agent answer" in console.output, deadline):
                return "the guest's agent key request got no answer"
        finally:
            control.stdin.close()
            control.wait()
        if b"Guest agent answer a person has control" not in console.output:
            return "agent input was not refused while a person held control"
        return None
    finally:
        end(sandbox)


def person(action: dict) -> dict:
    return {"account": PERSON, "action": action}


def host(directory: Path, request: dict) -> dict:
    """One request over its own host channel connection."""
    relay = connect(directory)
    try:
        return ask(relay, request)
    finally:
        relay.stdin.close()
        relay.wait()


def connect(directory: Path) -> subprocess.Popen:
    """A host channel connection, as the orchestrator opens one through exec."""
    return subprocess.Popen(
        ["sudo", "env", f"CUBEBOT_GUEST_DESKTOP_SOCKET={directory / 'desktop.sock'}"]
        + [ROOT / "target/debug/cubebot-sandbox", "desktop"],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
    )


def ask(relay: subprocess.Popen, request: dict) -> dict:
    relay.stdin.write(json.dumps(request).encode() + b"\n")
    relay.stdin.flush()
    return json.loads(relay.stdout.readline())


def rgb(encoded: str) -> bytes:
    """The pixels of an 8-bit RGB PNG whose rows use filters None or Up only."""
    png = base64.b64decode(encoded)
    if png[:8] != b"\x89PNG\r\n\x1a\n":
        return b""
    at, data, width = 8, b"", 0
    while at < len(png):
        (length,) = struct.unpack(">I", png[at : at + 4])
        kind, body = png[at + 4 : at + 8], png[at + 8 : at + 8 + length]
        if kind == b"IHDR":
            width = struct.unpack(">I", body[:4])[0]
        elif kind == b"IDAT":
            data += body
        at += 12 + length
    rows, stride = zlib.decompress(data), width * 3
    pixels, previous = bytearray(), bytes(stride)
    for start in range(0, len(rows), stride + 1):
        line = rows[start + 1 : start + 1 + stride]
        if rows[start] == 2:
            line = bytes((x + up) & 0xFF for x, up in zip(line, previous, strict=True))
        pixels += line
        previous = line
    return bytes(pixels)


def expected_rgb() -> bytes:
    bgrx = guest_desktop.pattern()
    rgb = bytearray(len(bgrx) // 4 * 3)
    rgb[0::3], rgb[1::3], rgb[2::3] = bgrx[2::4], bgrx[1::4], bgrx[0::4]
    return bytes(rgb)


def launch(variables: dict) -> subprocess.Popen:
    """Starts the sandbox in the pod namespace; sudo relays signals sent to it."""
    return subprocess.Popen(
        ["sudo", "ip", "netns", "exec", NAMESPACE, "env"]
        + [f"{name}={value}" for name, value in variables.items()]
        + [ROOT / "target/debug/cubebot-sandbox", "run"],
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
    )


def end(sandbox: subprocess.Popen):
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
        "CUBEBOT_GUEST_DESKTOP_SOCKET": directory / "desktop.sock",
    } | SIZES


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
    cpus = f"Brought up 1 node, {VCPUS} CPU{'s' if VCPUS > 1 else ''}".encode()
    if cpus not in console.output:
        return f"the guest did not bring up {VCPUS} vCPUs"
    for line, failure in [
        (b"Runtime startup failed", "the agent could not start, or open its journal"),
        (b"APIC ID mismatch", "the guest found its CPUID and MP table APIC IDs differ"),
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
