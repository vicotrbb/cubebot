# The guest kernel and root image from tooling/guest, passed as `--build-context guest=target/guest`.
# Without that context this empty stage stands in and the COPY of bzImage and root.ext4 below fails.
FROM scratch AS guest

FROM rust:1.96.0-bookworm AS build
WORKDIR /src
ENV CARGO_INCREMENTAL=0
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY apps/orchestrator ./apps/orchestrator
COPY apps/agent ./apps/agent
COPY apps/sandbox ./apps/sandbox
COPY modules ./modules
# Cached registry and target keep compiled dependencies across source changes; the
# mount is not part of the image, so the binaries are copied out to /out.
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --locked --release -p cubebot-agent -p cubebot-sandbox --bins \
    && mkdir /out \
    && cp target/release/cubebot-agent target/release/cubebot-sandbox /out/

# One container per agent (docs/adr/sandbox.md#in-house-vmm-as-the-agent-sandbox). The sandbox reads its guest kernel and root image from
# CUBEBOT_GUEST_KERNEL and CUBEBOT_GUEST_ROOT; iproute2 joins its tap to the pod network and
# e2fsprogs formats a new workspace disk. Launchers run it as root with only NET_ADMIN (see the
# orchestrator's pod layout); the image's own default user stays unprivileged.
FROM debian:bookworm-slim AS sandbox
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates bash e2fsprogs iproute2 \
    && rm -rf /var/lib/apt/lists/*
# A new Docker volume copies this owner and mode, so a sandbox in group 10001 can write its workspace.
RUN install -d -m 0775 -o 10001 -g 10001 /var/lib/cubebot
# Before the binaries, so a build without the guest fails without waiting for cargo.
COPY --from=guest bzImage root.ext4 /usr/share/cubebot/guest/
COPY --from=build /out/cubebot-agent /out/cubebot-sandbox /usr/local/bin/
ENV CUBEBOT_GUEST_KERNEL=/usr/share/cubebot/guest/bzImage \
    CUBEBOT_GUEST_ROOT=/usr/share/cubebot/guest/root.ext4
USER 10001:10001
ENTRYPOINT ["/usr/local/bin/cubebot-sandbox"]
CMD ["run"]
# Release builds pass these (.github/workflows/release.yml); last, so they never invalidate a cached layer.
ARG CUBEBOT_VERSION=dev
ARG CUBEBOT_REVISION=unknown
ARG CUBEBOT_CREATED=unknown
LABEL org.opencontainers.image.title="Cubebot sandbox" \
      org.opencontainers.image.description="The Cubebot in-house VMM, which runs one agent in a KVM guest." \
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
