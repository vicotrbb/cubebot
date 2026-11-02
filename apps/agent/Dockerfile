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

# The kvm guest's read-only root filesystem (ADR 0084). tooling/guest exports it to ext4, and the
# kernel runs `cubebot-agent guest` as init, which mounts the workspace disk on /var/lib/cubebot.
FROM debian:bookworm-slim
# CUBEBOT_TOOL_USER is 10001:10001; guest init creates and chowns its home on the workspace disk.
# The desktop (ADR 0085): guest init starts Xvfb, the outer display only the arbiter reaches, and
# Xephyr nested on it without XTEST as the tool user's display, when computer use is on; then
# openbox and xwallpaper, which draws wallpaper.jpg on the root window, as the tool user. The
# agent starts Chromium as the browser user, 10002, whose profile in /var/lib/cubebot/browser the
# tool user cannot read.
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates \
        git curl python3 build-essential ripgrep jq openssh-client \
        xvfb xserver-xephyr openbox xwallpaper chromium \
        xfonts-base fonts-dejavu-core \
    && rm -rf /var/lib/apt/lists/* \
    && install -d -m 0755 /var/lib/cubebot \
    && groupadd -g 10001 tool && useradd -u 10001 -g 10001 -M -d /var/lib/cubebot/workspace -s /bin/bash tool \
    && groupadd -g 10002 browser \
    && useradd -u 10002 -g 10002 -M -d /var/lib/cubebot/browser -s /usr/sbin/nologin browser
COPY apps/agent/wallpaper.jpg /usr/share/cubebot/wallpaper.jpg
# A person who logs in for the agent (ADR 0039) must not leave the password in the profile on the
# workspace disk, which the agent's tools can read; a managed policy on the read-only root holds.
COPY apps/agent/chromium-policy.json /etc/chromium/policies/managed/cubebot.json
COPY --from=build /out/cubebot-agent /usr/local/bin/
