FROM rust:1.96.0-bookworm AS build
WORKDIR /src
ENV CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=1
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY apps/orchestrator ./apps/orchestrator
COPY apps/agent ./apps/agent
COPY modules ./modules
RUN cargo build --locked --release -p cubebot-agent --bins --example isolation-probe

FROM debian:bookworm-slim AS base
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*

FROM base AS runtime-image
COPY --from=build /src/target/release/cubebot-agent /usr/local/bin/
USER 10001:10001
ENTRYPOINT ["/usr/local/bin/cubebot-agent"]

FROM base AS tools
RUN install -d -m 0700 -o 10002 -g 10002 /home/agent
COPY --from=build /src/target/release/cubebot-tool-runner /usr/local/bin/
USER 10002:10002
ENV HOME=/home/agent
WORKDIR /home/agent
ENTRYPOINT ["/usr/local/bin/cubebot-tool-runner"]

# Only the disposable qualification fixture includes kernel-policy tooling/probes.
FROM runtime-image AS qualification
USER 0:0
RUN apt-get update && apt-get install -y --no-install-recommends nftables iproute2 \
    && rm -rf /var/lib/apt/lists/*
COPY --from=build /src/target/release/examples/isolation-probe /usr/local/bin/
USER 10001:10001
ENTRYPOINT ["/usr/local/bin/isolation-probe"]

# An unqualified default build produces the runtime role, never the test fixture.
FROM runtime-image AS runtime
