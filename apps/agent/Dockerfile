FROM rust:1.96.0-bookworm AS build
WORKDIR /src
ENV CARGO_INCREMENTAL=0
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY apps/orchestrator ./apps/orchestrator
COPY apps/agent ./apps/agent
COPY apps/sandbox ./apps/sandbox
COPY modules ./modules
# The cache mount is not part of the image, so the binary is copied out to /out.
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --locked --release -p cubebot-agent --bin cubebot-agent \
    && install -D target/release/cubebot-agent /out/cubebot-agent

# One image for both runtimes: kvm's read-only root (init is `cubebot-agent guest`), or run as `cubebot-agent container`.
FROM debian:bookworm-slim
# Xvfb is the outer display only the arbiter reaches; Xephyr, nested without XTEST, is the tool user's.
# Chromium runs as the browser user, 10002, whose profile the tool user cannot read.
# The tool user is not root, so pip and npm install into ~/.local.
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
# A person's login must not leave a password in the profile on the workspace disk, which the agent's tools can read.
COPY apps/agent/chromium-policy.json /etc/chromium/policies/managed/cubebot.json
COPY apps/agent/desktop/chromium-flags /etc/chromium.d/cubebot
COPY apps/agent/desktop/tint2rc /etc/xdg/tint2/tint2rc
COPY apps/agent/desktop/pip.conf /etc/pip.conf
COPY apps/agent/desktop/npmrc /etc/npmrc
COPY --from=build /out/cubebot-agent /usr/local/bin/
# Last, so they never invalidate a cached layer.
ARG CUBEBOT_VERSION=dev
ARG CUBEBOT_REVISION=unknown
ARG CUBEBOT_CREATED=unknown
LABEL org.opencontainers.image.title="Cubebot agent" \
      org.opencontainers.image.description="One persistent Cubebot agent: its Rust runtime, tools and desktop, run directly in the container runtime or as the root filesystem of a KVM guest." \
      org.opencontainers.image.url="https://cubebot.io" \
      org.opencontainers.image.source="https://github.com/vicotrbb/cubebot" \
      org.opencontainers.image.documentation="https://github.com/vicotrbb/cubebot/blob/main/README.md" \
      org.opencontainers.image.vendor="Cubebot" \
      org.opencontainers.image.licenses="Apache-2.0" \
      org.opencontainers.image.base.name="docker.io/library/debian:bookworm-slim" \
      org.opencontainers.image.version="$CUBEBOT_VERSION" \
      org.opencontainers.image.revision="$CUBEBOT_REVISION" \
      org.opencontainers.image.created="$CUBEBOT_CREATED" \
      dev.orbstack.icon="https://raw.githubusercontent.com/vicotrbb/cubebot/main/apps/web/public/icons/icon-512.png"
