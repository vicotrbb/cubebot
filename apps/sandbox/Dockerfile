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

# One container per agent (ADR 0084). The kvm backend will add the guest kernel and root image.
FROM debian:bookworm-slim AS sandbox
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates bash \
    && rm -rf /var/lib/apt/lists/*
# A new Docker volume copies this owner, so the unprivileged sandbox can write its workspace.
RUN install -d -m 0755 -o 10001 -g 10001 /var/lib/cubebot
COPY --from=build /out/cubebot-agent /out/cubebot-sandbox /usr/local/bin/
USER 10001:10001
ENTRYPOINT ["/usr/local/bin/cubebot-sandbox"]
CMD ["--backend", "kvm"]
