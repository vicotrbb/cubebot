#!/usr/bin/python3 -u
"""Init of guest-boot's desktop boot (ADR 0085), in place of the agent.

Draws `pattern()` on the 1280x800 mode through a DRM dumb buffer, prints "Guest
desktop drawn", then prints the first key the virtio keyboard's evdev device reports
pressed. The host types it only after a person takes control, so the agent's own key
request that follows must be refused; the script prints the arbiter's answer and exits,
which ends the VM. guest_boot.py imports `pattern()` to check the host's screen.
"""

import ctypes
import fcntl
import glob
import json
import mmap
import os
import socket
import struct

WIDTH, HEIGHT = 1280, 800
MODE = "<I10H3I32s"  # struct drm_mode_modeinfo, 68 bytes
EVENT = "<qqHHi"  # struct input_event on x86_64


def pattern() -> bytes:
    """BGRX rows: blue 0x80, green the row and red the column, each modulo 256."""
    row = bytearray(b"\x80\x00\x00\x00" * WIDTH)
    row[2::4] = bytes(x & 0xFF for x in range(WIDTH))
    rows = []
    for y in range(HEIGHT):
        row[1::4] = bytes([y & 0xFF]) * WIDTH
        rows.append(bytes(row))
    return b"".join(rows)


def ioctl(fd: int, nr: int, layout: str, *values, kind: str = "d"):
    """An _IOWR ioctl on a packed struct; returns its fields as the kernel left them."""
    buffer = bytearray(struct.pack(layout, *values))
    request = 3 << 30 | len(buffer) << 16 | ord(kind) << 8 | nr
    fcntl.ioctl(fd, request, buffer)
    return struct.unpack(layout, buffer)


def address(array) -> int:
    return ctypes.addressof(array)


def draw(card: int):
    card_res, get_connector = "<4Q8I", "<4Q12I"
    counts = ioctl(card, 0xA0, card_res, *[0] * 12)
    crtcs = (ctypes.c_uint32 * counts[5])()
    connectors = (ctypes.c_uint32 * counts[6])()
    pointers = [0, address(crtcs), address(connectors), 0]
    ioctl(card, 0xA0, card_res, *pointers, 0, *counts[5:7], *[0] * 5)
    request = [0] * 8 + [connectors[0]] + [0] * 7
    count = ioctl(card, 0xA7, get_connector, *request)[4]
    modes = ctypes.create_string_buffer(count * 68)
    request[1], request[4] = address(modes), count
    ioctl(card, 0xA7, get_connector, *request)
    mode = next(
        raw
        for raw in (modes.raw[at : at + 68] for at in range(0, count * 68, 68))
        if struct.unpack(MODE, raw)[1:7:5] == (WIDTH, HEIGHT)
    )
    dumb = ioctl(card, 0xB2, "<6IQ", HEIGHT, WIDTH, 32, 0, 0, 0, 0)
    handle, pitch, size = dumb[4:]
    fb_cmd = (0, WIDTH, HEIGHT, pitch, 32, 24, handle)
    framebuffer = ioctl(card, 0xAE, "<7I", *fb_cmd)[0]
    offset = ioctl(card, 0xB3, "<IIQ", handle, 0, 0)[2]
    access = mmap.PROT_READ | mmap.PROT_WRITE
    frame = mmap.mmap(card, size, mmap.MAP_SHARED, access, offset=offset)
    image, line = pattern(), WIDTH * 4
    for y in range(HEIGHT):
        frame[y * pitch : y * pitch + line] = image[y * line : (y + 1) * line]
    # SETCRTC damages the whole plane, so the driver transfers and flushes all of it.
    crtc = (address(connectors), 1, crtcs[0], framebuffer, 0, 0, 0, 1, mode)
    ioctl(card, 0xA2, "<Q7I68s", *crtc)


def keyboard() -> int:
    for path in sorted(glob.glob("/dev/input/event*")):
        fd = os.open(path, os.O_RDONLY)
        name = fcntl.ioctl(fd, 2 << 30 | 256 << 16 | ord("E") << 8 | 0x06, bytes(256))
        if name.rstrip(b"\0") == b"cubebot keyboard":
            return fd
        os.close(fd)
    raise SystemExit("Guest desktop found no virtio keyboard")


def main():
    card = os.open("/dev/dri/card0", os.O_RDWR)
    # Opened before the host is told to type, so no key press goes unread.
    events = keyboard()
    draw(card)
    print("Guest desktop drawn")
    while True:
        _, _, kind, code, value = struct.unpack(
            EVENT, os.read(events, struct.calcsize(EVENT))
        )
        if (kind, value) == (1, 1):
            print(f"Guest key {code} pressed")
            break
    print(f"Guest agent answer {agent_key()}")


def agent_key() -> str:
    """Asks for a key press on the agent's desktop port, as the agent does."""
    stream = socket.socket(socket.AF_VSOCK, socket.SOCK_STREAM)
    stream.connect((2, 1025))
    request = {
        "call": "00000000-0000-0000-0000-000000000001",
        "action": {"action": "key", "keys": ["b"]},
    }
    stream.sendall(json.dumps(request).encode() + b"\n")
    answer = stream.makefile("rb").readline()
    return json.loads(answer).get("error", "accepted")


if __name__ == "__main__":
    main()
