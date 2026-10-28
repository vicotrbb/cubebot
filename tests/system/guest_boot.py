"""Boot the kvm backend: agent startup line within 2 s, then a clean VM exit."""

import os
import platform
import selectors
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
GUEST = ROOT / "target/guest"
STARTUP = b"cubebot-agent "
DEADLINE = 2.0
SHUTDOWN = 10.0


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
    environment = {
        **os.environ,
        "CUBEBOT_GUEST_KERNEL": str(GUEST / "bzImage"),
        "CUBEBOT_GUEST_INITRAMFS": str(GUEST / "initramfs.cpio"),
    }
    started = time.monotonic()
    sandbox = subprocess.Popen(
        [ROOT / "target/debug/cubebot-sandbox", "--backend", "kvm"],
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        env=environment,
    )
    try:
        return check(sandbox, started)
    finally:
        sandbox.kill()
        sandbox.wait()


def check(sandbox: subprocess.Popen, started: float) -> int:
    output, seen = relay(sandbox, started + DEADLINE, b"", STARTUP)
    if not seen:
        print(f"\nguest-boot failed: no agent startup line within {DEADLINE}s")
        return 1
    print(f"\nguest-boot: agent started in {time.monotonic() - started:.2f}s")
    # Unconfigured, the agent exits; the guest then reboots, which must end the VM.
    _, closed = relay(sandbox, started + SHUTDOWN, output, None)
    if not closed or sandbox.wait(timeout=1) != 0:
        print(f"guest-boot failed: the VM did not exit cleanly within {SHUTDOWN}s")
        return 1
    print("guest-boot: guest shutdown ended the VM")
    return 0


def relay(sandbox: subprocess.Popen, deadline: float, output: bytes, until):
    """Echo guest output until `until` appears, or until EOF when it is None."""
    with selectors.DefaultSelector() as selector:
        selector.register(sandbox.stdout, selectors.EVENT_READ)
        while until is None or until not in output:
            remaining = deadline - time.monotonic()
            if remaining <= 0 or not selector.select(remaining):
                return output, False
            chunk = os.read(sandbox.stdout.fileno(), 65536)
            if not chunk:
                return output, until is None
            sys.stdout.buffer.write(chunk)
            sys.stdout.flush()
            output += chunk
    return output, True


if __name__ == "__main__":
    sys.exit(main())
