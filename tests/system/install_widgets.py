"""deploy/install.sh's picker and confirmation driven through a pty; needs no Docker."""

import fcntl
import os
import select
import shutil
import struct
import subprocess
import sys
import termios
import time
from pathlib import Path

INSTALLER = Path(__file__).resolve().parents[2] / "deploy/install.sh"
CHOICES = "!kvm=KVM guests;container=Containers;off=No agents"
SCRIPT = f"""
source {INSTALLER}
restored() {{
    stty -a <&3 | grep -Eq '(^|[[:space:]])-(echo|icanon)([[:space:]]|$)' ||
        echo "RESTORED $1"
}}
setup_terminal
trap 'cleanup; restored at-exit' EXIT
pick runtime "Where should agents run?" container "Explanation." "{CHOICES}"
restored after-pick
echo "PICKED=$runtime"
ask typed "Password" "" "Typed twice." secret
echo "SECRET=$typed"
confirm "Install?" y && echo FIRST=yes || echo FIRST=no
confirm "Install?" y && echo SECOND=yes || echo SECOND=no
restored after-confirm
"""
DOWN, UP = "\x1b[B", "\x1b[A"


class Terminal:
    def __init__(self, bash, script, environment=os.environ):
        self.master, self.slave = os.openpty()
        fcntl.ioctl(self.slave, termios.TIOCSWINSZ, struct.pack("HHHH", 30, 100, 0, 0))
        self.process = subprocess.Popen(
            [bash, "-c", script],
            stdin=self.slave,
            stdout=self.slave,
            stderr=self.slave,
            env=environment | {"TERM": "xterm-256color", "NO_COLOR": ""},
            preexec_fn=self.control,
        )
        self.output = b""
        self.seen = 0

    def control(self):
        os.setsid()
        fcntl.ioctl(0, termios.TIOCSCTTY, 0)

    def read_until(self, text, timeout=10):
        deadline = time.monotonic() + timeout
        while (found := self.output.find(text.encode(), self.seen)) < 0:
            remaining = deadline - time.monotonic()
            assert remaining > 0, f"no {text!r} in {self.output[self.seen :]!r}"
            if select.select([self.master], [], [], remaining)[0]:
                self.output += os.read(self.master, 4096)
        self.seen = found + len(text)

    def send(self, keys):
        os.write(self.master, keys.encode())

    def finish(self):
        # Drained while waiting: bash on macOS exits only once its output is read.
        deadline = time.monotonic() + 10
        while self.process.poll() is None:
            assert time.monotonic() < deadline, f"bash still runs: {self.output!r}"
            if select.select([self.master], [], [], 0.1)[0]:
                try:
                    self.output += os.read(self.master, 4096)
                except OSError:
                    pass
        code = self.process.returncode
        os.close(self.master)
        os.close(self.slave)
        return code


def choose(bash):
    terminal = Terminal(bash, SCRIPT)
    terminal.read_until("Enter")
    # Down to off, up to containers, up again stays: KVM is not on this machine; j, k,
    # then 3 to off, and 1 does nothing for the same reason.
    terminal.send(DOWN + UP + UP + "jk" + "31" + "\r")
    terminal.read_until("PICKED=")
    terminal.read_until("\u203a")
    terminal.send("hunter2\r")
    terminal.read_until("again")
    terminal.send("hunter2\r")
    terminal.read_until("SECRET=hunter2")
    terminal.read_until("[Y/n]")
    terminal.send("n")
    terminal.read_until("FIRST=")
    terminal.read_until("[Y/n]")
    terminal.send("\r")
    assert terminal.finish() == 0, terminal.output
    output = terminal.output.decode()
    assert "RESTORED at-exit" in output, "the terminal stayed raw"
    assert "RESTORED after-pick" in output, output
    assert "RESTORED after-confirm" in output, output
    assert "PICKED=off" in output, output
    assert "FIRST=no" in output and "SECOND=yes" in output, output
    assert output.count("hunter2") == 1, "the password was shown as typed"
    assert "\x1b[0m\x1b[?25h" in output.rsplit("SECOND", 1)[-1], repr(output[-80:])
    print(
        f"PASS {bash}: arrows, j, k, digits and Enter picked; confirm took n and Enter"
    )


def interrupt(bash):
    terminal = Terminal(bash, SCRIPT)
    terminal.read_until("Enter")
    terminal.send(DOWN)
    # As a person presses it: at the redrawn picker, not within the arrow's bytes.
    terminal.read_until("Enter")
    time.sleep(0.2)
    terminal.send("\x03")
    assert terminal.finish() == 130, terminal.output
    assert b"RESTORED at-exit" in terminal.output, "the terminal stayed raw"
    assert b"\x1b[?25h" in terminal.output.rsplit(b"Enter", 1)[-1], terminal.output
    print(f"PASS {bash}: Ctrl-C left the picker with the cursor and terminal restored")


def main():
    shells = {path for path in ("/bin/bash", shutil.which("bash")) if path}
    for bash in sorted(shells):
        choose(bash)
        interrupt(bash)


if __name__ == "__main__":
    sys.exit(main())
