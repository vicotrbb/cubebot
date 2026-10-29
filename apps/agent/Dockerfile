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
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && install -d -m 0755 /var/lib/cubebot
COPY --from=build /out/cubebot-agent /usr/local/bin/
