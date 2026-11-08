FROM rust:1.96.0-bookworm AS build
WORKDIR /src
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY apps/orchestrator ./apps/orchestrator
COPY apps/agent ./apps/agent
COPY apps/sandbox ./apps/sandbox
COPY modules ./modules
RUN cargo build --locked --release -p cubebot-orchestrator

FROM debian:bookworm-slim AS runtime
# Backups run pg_dump and pg_restore, which must be at least as new as the server: PostgreSQL 18's
# client from the PostgreSQL project's repository, which Debian bookworm does not carry.
ADD https://www.postgresql.org/media/keys/ACCC4CF8.asc /usr/share/keyrings/postgresql.asc
RUN chmod 644 /usr/share/keyrings/postgresql.asc \
    && apt-get update && apt-get install -y --no-install-recommends ca-certificates \
    && echo "deb [signed-by=/usr/share/keyrings/postgresql.asc] https://apt.postgresql.org/pub/repos/apt bookworm-pgdg main" \
        > /etc/apt/sources.list.d/pgdg.list \
    && apt-get update && apt-get install -y --no-install-recommends postgresql-client-18 \
    && rm -rf /var/lib/apt/lists/*
# Compose installations with agents start sandboxes through the host's Docker CLI; Kubernetes never runs it.
COPY --from=docker:28.5.1-cli /usr/local/bin/docker /usr/local/bin/docker
COPY --from=build /src/target/release/cubebot-orchestrator /src/target/release/cubebot-admin /usr/local/bin/
# A fresh named volume mounted here takes this owner, so the orchestrator can write shared files.
RUN install -d -o 65532 -g 65532 /var/lib/cubebot/files
USER 65532:65532
ENV CUBEBOT_BIND_ADDR=0.0.0.0:8080
EXPOSE 8080
ENTRYPOINT ["cubebot-orchestrator"]
# Last, so they never invalidate a cached layer.
ARG CUBEBOT_VERSION=dev
ARG CUBEBOT_REVISION=unknown
ARG CUBEBOT_CREATED=unknown
LABEL org.opencontainers.image.title="Cubebot orchestrator" \
      org.opencontainers.image.description="The Cubebot control plane: agents, conversations, rooms, routines, files and the HTTP API, with PostgreSQL as its source of truth." \
      org.opencontainers.image.url="https://cubebot.io" \
      org.opencontainers.image.source="https://github.com/vicotrbb/cubebot" \
      org.opencontainers.image.documentation="https://github.com/vicotrbb/cubebot/blob/main/README.md" \
      org.opencontainers.image.vendor="Cubebot" \
      org.opencontainers.image.licenses="Apache-2.0" \
      org.opencontainers.image.base.name="docker.io/library/debian:bookworm-slim" \
      org.opencontainers.image.version="$CUBEBOT_VERSION" \
      org.opencontainers.image.revision="$CUBEBOT_REVISION" \
      org.opencontainers.image.created="$CUBEBOT_CREATED" \
      dev.orbstack.icon="https://raw.githubusercontent.com/vicotrbb/cubebot/main/apps/web/public/icons/container-orchestrator.png"
