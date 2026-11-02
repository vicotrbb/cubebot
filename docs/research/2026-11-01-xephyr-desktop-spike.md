# Xephyr desktop spike

Spike date: November 1, 2026. This note covers step 1 of the [portable sandbox design](../superpowers/specs/2026-11-01-portable-sandbox-design.md) and checks decision 1 there: a root-owned outer Xvfb that only the arbiter can reach, with Xephyr nested on it as the tool user's desktop and XTEST turned off. The spike was throwaway code in a Debian container outside the repo. It used Python with `python3-xlib`, not the Rust arbiter. No `src/` file changed. The container ran under Docker Desktop on an Apple Silicon Mac (aarch64), not in a KVM guest. It ran with Docker's default seccomp profile, and Chromium had `--no-sandbox`. None of this qualifies desktop security or agent behavior under ADR 0085.

## Result

The four checks passed, so the design's decision 1 holds on this evidence.

| Check | Result |
|---|---|
| (a) XTEST on `:0` reaches a window on `:1` | Passed. A motion and click sent to `:0` at root (980, 355) arrived at window coordinates (100, 60) with `send_event` false. Typing "hello" as XTEST key events arrived as the keysyms `hello`. A second test clicked Chromium's address bar through `:0`, sent Ctrl+A, and typed `typed via xtest on :0` including a Shift+colon. Chromium showed the exact text, so XI2 clients receive the input as well as core-event clients. |
| (b) GetImage of `:0`'s root | Passed. A 1280x800 ZPixmap of `:0`'s root shows the wallpaper, openbox's decorations, the Chromium window and the test window. Xephyr's window is the only top-level window on `:0`, at (0, 0) and 1280x800, so `:0` coordinates match `:1` coordinates one to one. |
| (c) uid 10001 cannot reach `:0`, and XTEST is refused on `:1` | Passed. As uid 10001: reading the root cookie fails with `EACCES`, connecting to `/tmp/.X11-unix/X0` fails with `EACCES`, the abstract socket `@/tmp/.X11-unix/X0` and TCP 6000 both refuse the connection, and `Display(":0")` fails. `/proc/net/unix` lists no abstract X sockets. On `:1`, `QueryExtension("XTEST")` reports the extension absent, and the server's extension list has no XTEST. |
| (d) Typing latency | Measured from the arbiter's XTEST `KeyPress` on `:0` (taken just before the flush) to the `KeyPress` reaching the client on `:1`, using the system-wide monotonic clock and 200 characters sent one at a time: min 0.06 ms, median 0.29 ms, p95 1.65 ms, max 3.15 ms. In a burst of 500 press/release pairs sent with no waiting, all 500 presses arrived, the last 27.8 ms after the first was sent (0.056 ms per character). Xephyr adds no latency that matters for computer use. |

## Findings for step 2

- **Turn off the abstract socket.** On Linux the X server also listens on the abstract socket `@/tmp/.X11-unix/X<n>` by default. File permissions do not cover that socket, and any process in the network namespace can connect to it. `-nolisten local` turns it off. The arbiter must pass `-nolisten tcp -nolisten local` to the outer server. With that, the 0700 socket mode is real protection.
- **The socket mode is set after start.** Xvfb creates `/tmp/.X11-unix/X0` with mode 0777. The spike used `chmod 0700` once the socket appeared, which leaves a short window. Two things close it: the server requires a root-only `-auth` cookie, and init starts the outer server before any tool-user process exists. Keep both.
- **XSendEvent still works on `:1`.** Removing XTEST does not stop a tool-user client from sending synthetic events to another client's window with `XSendEvent`. The spike's synthetic `KeyPress` arrived with `send_event` set to true. Chromium ignores synthetic key events on its windows, but other X clients may accept them. Core `WarpPointer` also still moves the pointer on `:1`. Both were already possible on today's single-server desktop, so this is not a regression. The ADR 0085 amendment should name them: the tool user cannot produce device events, but it can still send synthetic events, which carry a flag.
- **Inner extensions.** `:1` offers BIG-REQUESTS, Composite, DAMAGE, DOUBLE-BUFFER, GLX, Generic Event Extension, MIT-SCREEN-SAVER, MIT-SHM, Present, RANDR, RENDER, SECURITY, SHAPE, SYNC, X-Resource, XC-MISC, XFIXES, XINERAMA, XInputExtension, XKEYBOARD and XVideo. XInput 2 has no device-injection request, so XInputExtension is not a way back to XTEST.
- **Focus.** `:0` has no window manager. Keys go to Xephyr only because focus is PointerRoot and the pointer is over Xephyr's full-screen window. The arbiter should `SetInputFocus` to Xephyr's window on `:0` at startup and not depend on where the pointer is.
- **Keymaps match.** Xvfb and Xephyr both start with the default XKB `evdev`/`us` keymap, so an arbiter that looks up keycodes on `:0` sends keys that resolve to the same keysyms on `:1`. A non-default layout on either server would break this.
- **`-no-host-grab`** stops Xephyr's Ctrl+Shift grab toggle from catching injected key chords.

## Package versions

Debian GNU/Linux 12 (bookworm), aarch64, `debian:bookworm-slim` pulled on November 1, 2026.

| Package | Version |
|---|---|
| xvfb, xserver-xephyr, xserver-common | 2:21.1.7-3+deb12u13 |
| chromium | 156.0.8078.73-1~deb12u1 |
| openbox | 3.6.1-10 |
| xwallpaper | 0.7.3-1+b1 |
| x11-utils | 7.7+5 |
| python3 | 3.11.2-1+b1 |
| python3-xlib | 0.33-2 |
| libc6 | 2.36-9+deb12u14 |

## Exact commands

Image: the runtime package list from `apps/agent/Dockerfile`, plus `xvfb xserver-xephyr x11-utils xauth python3-xlib python3-pil procps`, a `tool` user with uid and gid 10001, and `apps/agent/wallpaper.jpg` at `/usr/share/cubebot/wallpaper.jpg`.

```sh
docker build -t xephyr-spike .
docker run -d --name xs -v "$PWD/out:/out" xephyr-spike sleep infinity
docker exec xs sh /spike/start.sh
```

`start.sh`, run as root:

```sh
umask 077
mkdir -p /root/x
xauth -q -f /root/x/outer add :0 . "$(od -An -N16 -tx1 /dev/urandom | tr -d ' \n')"
Xvfb :0 -screen 0 1280x800x24 -auth /root/x/outer -nolisten tcp -nolisten local &
while [ ! -S /tmp/.X11-unix/X0 ]; do sleep 0.05; done
chmod 0700 /tmp/.X11-unix/X0
xauth -q -f /root/x/inner add :1 . "$(od -An -N16 -tx1 /dev/urandom | tr -d ' \n')"
install -o tool -g tool -m 0600 /root/x/inner /home/tool/.Xauthority
DISPLAY=:0 XAUTHORITY=/root/x/outer Xephyr :1 -screen 1280x800 -extension XTEST \
    -auth /root/x/inner -nolisten tcp -nolisten local -no-host-grab &
while [ ! -S /tmp/.X11-unix/X1 ]; do sleep 0.05; done
T="runuser -u tool -- env DISPLAY=:1 XAUTHORITY=/home/tool/.Xauthority HOME=/home/tool"
$T openbox &
$T xwallpaper --zoom /usr/share/cubebot/wallpaper.jpg
$T chromium --no-sandbox --disable-dev-shm-usage --no-first-run \
    --window-position=40,40 --window-size=760,560 'data:text/html,...' &
```

Result: `X0` is `srwx------ root root` and `X1` is `srwxrwxrwx root root`. The tool user's access to `:1` depends on the cookie, not on the socket mode.

Probes:

```sh
docker exec -d -u tool -e XAUTHORITY=/home/tool/.Xauthority xs python3 /spike/receiver.py  # window on :1, logs events with monotonic ns
docker exec xs python3 /spike/probe.py                                                     # (a), (b), (d) as root on :0
docker exec -u tool -e XAUTHORITY=/home/tool/.Xauthority xs python3 /spike/negative.py     # (c) as uid 10001
docker exec xs python3 /spike/chromium_type.py                                             # (a) against Chromium's address bar
```

- `receiver.py` maps a 320x220 window on `:1` and focuses it. It writes the window's root position to `/tmp/win.json`, then logs each KeyPress, KeyRelease, ButtonPress and MotionNotify as JSON with `time.monotonic_ns()`, the keysym and the `send_event` flag.
- `probe.py` asserts that XTEST is present on `:0` and lists `:0`'s top-level windows. It sends `xtest.fake_input` motion, click and keys, then checks the receiver's log. It saves `root.get_image(0, 0, 1280, 800, ZPixmap)` as a PNG and runs the serial and burst latency loops.
- `negative.py` tries the cookie, the filesystem, abstract and TCP sockets, and `Display(":0")`, and expects each to fail. It checks `has_extension("XTEST")` on `:1`, then sends one `XSendEvent` KeyPress to the receiver.

## Limits

- The spike ran on aarch64 under Docker Desktop's Linux VM, not on amd64 and not inside a KVM guest. The latency figures cover one idle machine.
- The XTEST refusal on `:1` was shown by the server reporting the extension absent. `python3-xlib` will not encode a request for an extension the server lacks, so no raw request with a made-up opcode was sent.
- Chromium ran as uid 10001, as the task asked, not as the browser user 10002 that ADR 0085 specifies. It also ran without its sandbox. Neither affects the X checks.
- The Rust arbiter, `x11rb`, PNG tiling and the protocol were not exercised.
