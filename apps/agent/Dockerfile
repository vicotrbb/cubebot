FROM rust:1.96.0-bookworm AS build
WORKDIR /src
ENV CARGO_INCREMENTAL=0
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY apps/orchestrator ./apps/orchestrator
COPY apps/agent ./apps/agent
COPY modules ./modules
# Cached registry and target keep compiled dependencies across source changes; the
# mount is not part of the image, so the binaries are copied out to /out.
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --locked --release -p cubebot-agent --bins --example isolation-probe \
    && mkdir /out \
    && cp target/release/cubebot-agent target/release/cubebot-tool-runner \
        target/release/examples/isolation-probe /out/

FROM debian:bookworm-slim AS base
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*

FROM base AS runtime-image
RUN apt-get update && apt-get install -y --no-install-recommends nftables \
    && rm -rf /var/lib/apt/lists/*
COPY --from=build /out/cubebot-agent /usr/local/bin/
USER 10001:10001
ENTRYPOINT ["/usr/local/bin/cubebot-agent"]

FROM base AS tools
RUN apt-get update && apt-get install -y --no-install-recommends util-linux bash \
    && rm -rf /var/lib/apt/lists/*
RUN install -d -m 0700 -o 10002 -g 10002 /home/agent
COPY --from=build /out/cubebot-tool-runner /usr/local/bin/
USER 10002:10002
ENV HOME=/home/agent
WORKDIR /home/agent
ENTRYPOINT ["/usr/local/bin/cubebot-tool-runner"]

# Only the disposable qualification fixture includes kernel-policy tooling/probes.
FROM runtime-image AS qualification
USER 0:0
RUN apt-get update && apt-get install -y --no-install-recommends nftables iproute2 util-linux bash \
    && rm -rf /var/lib/apt/lists/*
COPY --from=build /out/isolation-probe /usr/local/bin/
COPY --from=build /out/cubebot-tool-runner /usr/local/bin/
USER 10001:10001
ENTRYPOINT ["/usr/local/bin/isolation-probe"]

# An unqualified default build produces the runtime role, never the test fixture.
FROM runtime-image AS runtime
