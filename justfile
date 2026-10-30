set shell := ["bash", "-eu", "-o", "pipefail", "-c"]

default:
    @just --list

# Install exactly the locked dependency graph.
bootstrap:
    bun install --frozen-lockfile
    bun install --frozen-lockfile --cwd site
    uv sync --project tooling/quality --locked
    cargo fetch --locked
    cargo install rust-code-analysis-cli --version 0.0.25 --locked --root .tools

# Run these in separate terminals.
dev-api:
    cargo run --locked -p cubebot-orchestrator

dev-web:
    bun run dev

format:
    cargo fmt --all
    bun run format
    uv run --project tooling/quality --locked ruff format --config tooling/quality/pyproject.toml tooling tests

format-check:
    cargo fmt --all -- --check
    bun run format:check
    uv run --project tooling/quality --locked ruff format --config tooling/quality/pyproject.toml --check tooling tests

complexity:
    uv run --project tooling/quality --locked python tooling/quality/complexity.py

lint: complexity
    cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
    bun run lint
    uv run --project tooling/quality --locked ruff check --config tooling/quality/pyproject.toml tooling tests

test:
    bun test apps/web/src/api/client.test.ts
    cargo test --workspace --locked
    uv run --project tooling/quality --locked python -m unittest discover -s tooling/quality -p 'test_*.py'

build:
    cargo build --workspace --locked
    bun run build
    bun run --cwd site build

# Public landing site; static output in site/dist.
site-dev:
    bun run --cwd site dev

# Writes the orchestrator's browser API document and the web client's types generated from it.
api:
    cargo run --locked -q -p cubebot-orchestrator --bin cubebot-admin openapi > apps/web/src/api/openapi.json
    bun run --cwd apps/web api
    bun run prettier --log-level warn --write apps/web/src/api/openapi.json apps/web/src/api/schema.d.ts

# Building first generates the route tree on a fresh checkout.
check: format-check lint test build
    bun run typecheck

# Starts temporary local processes, checks HTTP behavior, and stops them.
smoke:
    uv run --project tooling/quality --locked python tests/system/smoke.py

# Builds the guest into target/guest and runs its acceptance in a network namespace (sudo); skips off x86_64 Linux.
guest-boot:
    uv run --project tooling/quality --locked python tests/system/guest_boot.py

db-up:
    docker compose --env-file .env up -d --wait postgres

# Retains the PostgreSQL volume.
db-down:
    docker compose --env-file .env down

# Reuses a running `just db-up` server, else a disposable one; never an application DB.
integration:
    uv run --project tooling/quality --locked python tests/system/integration.py

# cluster-* recipes run nightly in CI (.github/workflows/nightly.yml), not per push.

# Owns a disposable local cluster and PostgreSQL; never uses the default kubeconfig.
cluster-admission:
    uv run --project tooling/quality --locked python tests/system/cluster_admission.py

# Uses the shipped agent images and a test-only CSI backend in an owned cluster.
cluster-startup:
    uv run --project tooling/quality --locked python tests/system/cluster_startup.py

# Builds shipped images; requires an external evidence directory and provider key file.
cluster-chat:
    uv run --project tooling/quality --locked python tests/system/cluster_chat.py

# ADR 0083 sandbox scenarios on the kvm backend; x86_64 Linux with /dev/kvm, as cluster-chat.
cluster-sandbox:
    uv run --project tooling/quality --locked python tests/system/cluster_sandbox.py

# A self-signed control CA and a certificate for the docker0 address; kept once made, remove the directory to reissue.
dev-ca directory=".cubebot/dev-ca":
    #!/usr/bin/env bash
    set -euo pipefail
    umask 077
    mkdir -p "{{ directory }}" && cd "{{ directory }}"
    [ -f server.crt ] && exit 0
    bridge=$(ip -4 -o addr show docker0 | awk '{print $4}' | cut -d/ -f1)
    key=(-newkey ec -pkeyopt ec_paramgen_curve:prime256v1 -nodes)
    # Strict verifiers (Python 3.13+, for one) need key usage on both certificates.
    openssl req -x509 "${key[@]}" -keyout ca.key -out ca.crt -days 825 -subj "/CN=cubebot development control CA" \
        -addext "basicConstraints=critical,CA:TRUE" -addext "keyUsage=critical,keyCertSign,cRLSign"
    openssl req "${key[@]}" -keyout server.key -out server.csr -subj "/CN=$bridge"
    printf 'subjectAltName=IP:%s\nbasicConstraints=CA:FALSE\nkeyUsage=critical,digitalSignature\nextendedKeyUsage=serverAuth\nsubjectKeyIdentifier=hash\nauthorityKeyIdentifier=keyid\n' "$bridge" > server.ext
    openssl x509 -req -in server.csr -CA ca.crt -CAkey ca.key -CAcreateserial -out server.crt -days 825 -extfile server.ext
    rm server.csr server.ext
    # Sandbox containers run as root without CAP_DAC_OVERRIDE; the certificate is public.
    chmod 644 ca.crt

# Serves HTTPS on docker0:8443 for Docker sandboxes; needs `just db-up` and the cubebot-sandbox:dev image.
docker-orchestrator: dev-ca
    #!/usr/bin/env bash
    set -euo pipefail
    set -a; source .env; set +a
    bridge=$(ip -4 -o addr show docker0 | awk '{print $4}' | cut -d/ -f1)
    state="$PWD/.cubebot"
    [ -f "$state/credential.key" ] || (umask 077 && head -c 32 /dev/urandom > "$state/credential.key")
    printf '{"image":"cubebot-sandbox:dev","orchestrator_origin":"https://%s:8443","control_ca_file":"%s","terminal_enabled":true}\n' \
        "$bridge" "$state/dev-ca/ca.crt" > "$state/docker.json"
    CUBEBOT_DATABASE_URL="postgres://cubebot:$POSTGRES_PASSWORD@127.0.0.1:${POSTGRES_PORT:-5432}/cubebot" \
    CUBEBOT_BIND_ADDR="$bridge:8443" \
    CUBEBOT_TLS_CERT_FILE="$state/dev-ca/server.crt" CUBEBOT_TLS_KEY_FILE="$state/dev-ca/server.key" \
    CUBEBOT_CREDENTIAL_KEY_FILE="$state/credential.key" CUBEBOT_DOCKER_CONFIG="$state/docker.json" \
        cargo run --locked -p cubebot-orchestrator

# Against `just docker-orchestrator`: bootstraps the administrator and OpenAI provider once, then creates and wakes an agent.
docker-agent name:
    uv run --project tooling/quality --locked python tests/system/docker_sandbox.py agent "{{ name }}"

# Boots one agent through Docker and runs one bash tool call; x86_64 Linux with /dev/kvm and OPENAI_API_KEY_FILE.
docker-sandbox:
    uv run --project tooling/quality --locked python tests/system/docker_sandbox.py
