FROM rust:1.96.0-bookworm AS build
WORKDIR /src
ENV CARGO_INCREMENTAL=0
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY apps/orchestrator ./apps/orchestrator
COPY apps/agent ./apps/agent
COPY apps/sandbox ./apps/sandbox
COPY modules ./modules
# Cached registry and target keep compiled dependencies across source changes; the
# mount is not part of the image, so the binary is copied out to /out.
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --locked --release -p cubebot-agent --bin cubebot-agent \
    && install -D target/release/cubebot-agent /out/cubebot-agent

# One agent image for both sandbox runtimes (docs/adr/sandbox.md#in-house-vmm-as-the-agent-sandbox). In `kvm` it is the guest's read-only root
# filesystem: tooling/guest exports it to ext4, and the kernel runs `cubebot-agent guest` as init,
# which mounts the workspace disk on /var/lib/cubebot. In `container` launchers run it directly
# as `cubebot-agent container`, with the workspace volume mounted on /var/lib/cubebot.
FROM debian:bookworm-slim
# CUBEBOT_TOOL_USER is 10001:10001; init creates and chowns its home on the workspace volume.
# The desktop (docs/adr/sandbox.md#desktop-as-a-vmm-device): init starts Xvfb, the outer display only the arbiter reaches, and
# Xephyr nested on it without XTEST as the tool user's display, when computer use is on; then
# openbox, xwallpaper, which draws wallpaper.jpg on the root window, and tint2, the taskbar a
# person or the agent uses to open the browser, a terminal, files or an editor, as the tool user.
# The agent starts its own Chromium as the browser user, 10002, whose profile in
# /var/lib/cubebot/browser the tool user cannot read.
# The tool user is not root, so it installs software where it can write: pip and npm put
# packages in ~/.local (pip.conf, npmrc), which is on its PATH, as are virtual environments.
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates \
        git curl wget python3 python-is-python3 python3-pip python3-venv pipx \
        nodejs npm build-essential ripgrep jq openssh-client \
        unzip zip xz-utils file less nano procps sqlite3 xdg-utils \
        xvfb xserver-xephyr openbox xwallpaper chromium \
        tint2 lxterminal pcmanfm l3afpad \
        xfonts-base fonts-dejavu-core \
    && rm -rf /var/lib/apt/lists/* \
    && install -d -m 0755 /var/lib/cubebot \
    && groupadd -g 10001 tool && useradd -u 10001 -g 10001 -M -d /var/lib/cubebot/workspace -s /bin/bash tool \
    && groupadd -g 10002 browser \
    && useradd -u 10002 -g 10002 -M -d /var/lib/cubebot/browser -s /usr/sbin/nologin browser
COPY apps/agent/wallpaper.jpg /usr/share/cubebot/wallpaper.jpg
# A person who logs in for the agent (docs/adr/sandbox.md#live-browser-viewing-and-explicit-human-takeover) must not leave the password in the profile on the
# workspace disk, which the agent's tools can read; a managed policy on the read-only root holds.
COPY apps/agent/chromium-policy.json /etc/chromium/policies/managed/cubebot.json
COPY apps/agent/desktop/chromium-flags /etc/chromium.d/cubebot
COPY apps/agent/desktop/tint2rc /etc/xdg/tint2/tint2rc
COPY apps/agent/desktop/pip.conf /etc/pip.conf
COPY apps/agent/desktop/npmrc /etc/npmrc
COPY --from=build /out/cubebot-agent /usr/local/bin/
