FROM rust:1.96.0-bookworm AS build
WORKDIR /src
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY apps/orchestrator ./apps/orchestrator
COPY apps/agent ./apps/agent
COPY modules ./modules
RUN cargo build --locked --release -p cubebot-orchestrator

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*
COPY --from=build /src/target/release/cubebot-orchestrator /src/target/release/cubebot-admin /usr/local/bin/
USER 65532:65532
ENV CUBEBOT_BIND_ADDR=0.0.0.0:8080
EXPOSE 8080
ENTRYPOINT ["cubebot-orchestrator"]
