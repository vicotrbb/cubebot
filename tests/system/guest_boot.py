"""Boot the kvm sandbox backend; require the agent startup line within 2 seconds."""

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
        return wait_for_startup(sandbox, started)
    finally:
        sandbox.kill()
        sandbox.wait()


def wait_for_startup(sandbox: subprocess.Popen, started: float) -> int:
    output = b""
    with selectors.DefaultSelector() as selector:
        selector.register(sandbox.stdout, selectors.EVENT_READ)
        while (remaining := started + DEADLINE - time.monotonic()) > 0:
            if not selector.select(remaining):
                break
            chunk = os.read(sandbox.stdout.fileno(), 65536)
            if not chunk:
                break
            sys.stdout.buffer.write(chunk)
            sys.stdout.flush()
            output += chunk
            if STARTUP in output:
                elapsed = time.monotonic() - started
                print(f"\nguest-boot: agent started in {elapsed:.2f}s")
                return 0
    print(
        f"\nguest-boot failed: no agent startup line within {DEADLINE}s",
        file=sys.stderr,
    )
    return 1


if __name__ == "__main__":
    sys.exit(main())
