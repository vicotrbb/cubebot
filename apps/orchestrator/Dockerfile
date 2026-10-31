FROM rust:1.96.0-bookworm AS build
WORKDIR /src
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY apps/orchestrator ./apps/orchestrator
COPY apps/agent ./apps/agent
COPY apps/sandbox ./apps/sandbox
COPY modules ./modules
RUN cargo build --locked --release -p cubebot-orchestrator

FROM debian:bookworm-slim AS runtime
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*
COPY --from=build /src/target/release/cubebot-orchestrator /src/target/release/cubebot-admin /usr/local/bin/
USER 65532:65532
ENV CUBEBOT_BIND_ADDR=0.0.0.0:8080
EXPOSE 8080
ENTRYPOINT ["cubebot-orchestrator"]

# `just up` with agents: the orchestrator starts sandboxes through the host's Docker CLI.
FROM runtime AS agents
COPY --from=docker:28.5.1-cli /usr/local/bin/docker /usr/local/bin/docker

# The default target.
FROM runtime
