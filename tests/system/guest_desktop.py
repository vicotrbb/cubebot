#!/usr/bin/python3 -u
"""Init of guest-boot's desktop boot (docs/adr/sandbox.md#desktop-as-a-vmm-device),
wrapping the real guest init.

Runs the real init (`cubebot-agent-real guest`) as its child, so the guest starts its
desktop and arbiter as it does for an agent. Once the desktop is ready it checks, as the
tool user, that the outer display, the arbiter's socket and the host channel are out of
reach and that the tool user's display has no XTEST, printing "Guest isolation passed"
or why not. Then, still as the tool user, it maps a focused window on :1, prints "Guest
desktop ready" and prints the first key the window receives. The host types it only
after a person takes control, so the agent's own key request that follows, sent as root
on the arbiter's socket, must be refused; it prints the arbiter's answer and exits,
which ends the VM.
"""

import json
import os
import socket
import struct
import time

REAL_INIT = "/usr/local/bin/cubebot-agent-real"
READY = "/run/cubebot-desktop"
ARBITER = "/run/cubebot-desktop.sock"
TOOL_USER = 10001
KEY_PRESS, MAP_NOTIFY = 2, 19


def main():
    if os.fork() == 0:
        os.execv(REAL_INIT, [REAL_INIT, "guest"])
    deadline = time.monotonic() + 60
    while not os.path.exists(READY):
        if time.monotonic() > deadline:
            raise SystemExit("Guest desktop never became ready")
        time.sleep(0.1)
    tool = os.fork()
    if tool == 0:
        os.setgroups([])
        os.setgid(TOOL_USER)
        os.setuid(TOOL_USER)
        failures = isolation()
        print(
            f"Guest isolation failed: {failures}"
            if failures
            else "Guest isolation passed"
        )
        receive_key()
        os._exit(0)
    os.waitpid(tool, 0)
    print(f"Guest agent answer {agent_key()}")


def isolation() -> list:
    """What the tool user could reach that it must not, by the spike's check (c)."""
    failures = []

    def refused(name, attempt, *expected):
        try:
            attempt()
            failures.append(f"{name} succeeded")
        except expected:
            pass
        except OSError as error:
            failures.append(f"{name}: {error!r}")

    refused(
        "reading the outer cookie",
        lambda: read("/run/cubebot/display"),
        PermissionError,
    )
    refused("connecting to :0", lambda: unix("/tmp/.X11-unix/X0"), PermissionError)
    refused(
        "the abstract :0 socket",
        lambda: unix("\0/tmp/.X11-unix/X0"),
        ConnectionRefusedError,
    )
    # Refused, or unreachable when loopback is down; either way nothing listens.
    refused("TCP :0", lambda: socket.create_connection(("127.0.0.1", 6000), 2), OSError)
    with open("/proc/net/unix") as table:
        if "@/tmp/.X11-unix" in table.read():
            failures.append("an abstract X socket is listed")
    refused("the arbiter's socket", lambda: unix(ARBITER), PermissionError)
    refused("a reserved vsock port", reserved_vsock, PermissionError)
    if X11(1).extension(b"XTEST"):
        failures.append("XTEST is present on :1")
    return failures


def read(path: str):
    with open(path, "rb") as file:
        file.read()


def unix(path: str):
    stream = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    stream.connect(path)
    stream.close()


def reserved_vsock():
    """A host channel stream needs a reserved source port, which takes root."""
    stream = socket.socket(socket.AF_VSOCK, socket.SOCK_STREAM)
    stream.bind((socket.VMADDR_CID_ANY, 1023))
    stream.close()


class X11:
    """The few core X11 requests this test needs, unauthorized, as :1 allows."""

    def __init__(self, display: int):
        self.stream = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self.stream.connect(f"/tmp/.X11-unix/X{display}")
        self.stream.sendall(struct.pack("<BxHHHHxx", ord("l"), 11, 0, 0, 0))
        head = self.read(8)
        body = self.read(struct.unpack("<H", head[6:8])[0] * 4)
        if head[0] != 1:
            raise SystemExit(f"X connection refused: {body!r}")
        self.base = struct.unpack("<I", body[4:8])[0]
        vendor, formats = struct.unpack("<H", body[16:18])[0], body[21]
        screens = 32 + (vendor + 3) // 4 * 4 + 8 * formats
        self.root = struct.unpack("<I", body[screens : screens + 4])[0]

    def read(self, length: int) -> bytes:
        data = b""
        while len(data) < length:
            chunk = self.stream.recv(length - len(data))
            if not chunk:
                raise SystemExit("the X server closed the connection")
            data += chunk
        return data

    def packet(self) -> bytes:
        """One event, reply or error; replies here never carry extra data."""
        return self.read(32)

    def extension(self, name: bytes) -> bool:
        padded = name + b"\0" * (-len(name) % 4)
        length = 2 + len(padded) // 4
        self.stream.sendall(struct.pack("<BxHHxx", 98, length, len(name)) + padded)
        while (reply := self.packet())[0] != 1:
            pass
        return reply[8] == 1

    def window(self) -> int:
        """Maps a window that takes key presses, focused once mapped."""
        window = self.base | 1
        # CreateWindow with KeyPress and StructureNotify events, then MapWindow.
        fields = (1, 0, 9, window, self.root, 40, 40, 320, 220, 0, 1, 0, 0x800, 0x20001)
        create = struct.pack("<BBHIIhhHHHHIII", *fields)
        self.stream.sendall(create + struct.pack("<BxHI", 8, 2, window))
        while self.packet()[0] & 0x7F != MAP_NOTIFY:
            pass
        # SetInputFocus to the window, reverting to its parent.
        self.stream.sendall(struct.pack("<BBHII", 42, 2, 3, window, 0))
        return window


def receive_key():
    display = X11(1)
    display.window()
    print("Guest desktop ready")
    while (event := display.packet())[0] & 0x7F != KEY_PRESS:
        pass
    print(f"Guest key {event[1]} pressed")


def agent_key() -> str:
    """Asks for a key press on the arbiter's socket, as the agent does."""
    stream = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    stream.connect(ARBITER)
    request = {
        "call": "00000000-0000-0000-0000-000000000001",
        "action": {"action": "key", "keys": ["b"]},
    }
    stream.sendall(json.dumps(request).encode() + b"\n")
    answer = stream.makefile("rb").readline()
    return json.loads(answer).get("error", "accepted")


if __name__ == "__main__":
    main()
