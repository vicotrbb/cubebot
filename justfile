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
    shellcheck deploy/install.sh deploy/cubebot

test:
    bun test apps/web/src/api/client.test.ts apps/web/src/features/desktop/desktop.test.ts apps/web/src/features/platform/sandbox.test.ts apps/web/src/features/platform/models.test.ts apps/web/src/features/platform/subscription.test.ts apps/web/src/features/chat/mentions.test.ts apps/web/src/features/platform/audit.test.ts apps/web/src/features/agent/doing.test.ts apps/web/src/features/knowledge/tree.test.ts apps/web/src/features/chat/tokens.test.ts apps/web/src/features/files/viewer.test.ts apps/web/src/features/situation/format.test.ts apps/web/src/features/chat/work.test.ts apps/web/src/features/tasks/status.test.ts apps/web/src/features/platform/access.test.ts apps/web/src/components/framePage.test.ts apps/web/src/features/call/call.test.ts apps/web/src/features/chat/Markdown.test.tsx apps/web/src/app/stream.test.ts apps/web/src/features/knowledge/useSave.test.ts apps/web/src/app/title.test.ts apps/web/src/features/connections/model.test.ts
    cargo test --workspace --locked
    uv run --project tooling/quality --locked python -m unittest discover -s tooling/quality -p 'test_*.py'
    uv run --project tooling/quality --locked python tests/system/install_widgets.py

build:
    cargo build --workspace --locked
    bun run build
    bun run --cwd site build

# Public landing site; static output in site/dist.
site-dev:
    bun run --cwd site dev

# Writes the browser API document and the web client's generated types.
api:
    cargo run --locked -q -p cubebot-orchestrator --bin cubebot-admin openapi > apps/web/src/api/openapi.json
    bun run --cwd apps/web api
    bun run prettier --log-level warn --write apps/web/src/api/openapi.json apps/web/src/api/schema.d.ts

# Lints the chart and renders each runtime and database mode.
chart:
    helm lint --strict deploy/chart --values deploy/chart/ci/values.yaml
    helm template cubebot deploy/chart --values deploy/chart/ci/values.yaml > /dev/null
    helm template cubebot deploy/chart --values deploy/chart/ci/values.yaml --set sandbox.runtime=kvm,controlCA=pem > /dev/null
    helm template cubebot deploy/chart --values deploy/chart/ci/values.yaml --set externalDatabase.existingSecret=database > /dev/null
    helm template cubebot deploy/chart --values deploy/chart/ci/values.yaml --set credentials.existingSecret=key,postgres.existingSecret=database > /dev/null

# Building first generates the route tree on a fresh checkout.
check: format-check lint test build chart
    bun run typecheck

# Compares a release's digests.json with what its tag resolves to in GHCR; needs Docker and jq.
release-check file="digests.json":
    tag=$(jq -er .tag "{{ file }}"); \
    for image in cubebot-orchestrator cubebot-web cubebot-agent cubebot-sandbox; do \
        want=$(jq -er --arg image "$image" '.images[$image]' "{{ file }}"); \
        have=$(docker buildx imagetools inspect "ghcr.io/vicotrbb/$image:$tag" --format '{{{{json .Manifest}}' | jq -er .digest); \
        [ "$want" = "$have" ] || { echo "$image:$tag is $have in GHCR, $want in {{ file }}" >&2; exit 1; }; \
        echo "$image:$tag $have"; \
    done

# Installs this checkout from a local registry, twice (defaults, then pty answers); Docker only.
install-check:
    uv run --project tooling/quality --locked python tests/system/install_check.py

# Starts temporary local processes, checks HTTP behavior, and stops them.
smoke:
    uv run --project tooling/quality --locked python tests/system/smoke.py

# The embedded speech models on real audio; downloads about 450 MB once.
speech-check directory="target/speech-models":
    cargo run --locked --release -p cubebot-orchestrator --example speech-check -- "{{ directory }}"

# A call in real Chromium with a recorded voice and a scripted model; Docker and Playwright (CUBEBOT_PLAYWRIGHT_MODULE).
call-check:
    uv run --project tooling/quality --locked python tests/system/call_check.py

# Builds the guest into target/guest and runs its acceptance in a network namespace (sudo); skips off x86_64 Linux.
guest-boot:
    uv run --project tooling/quality --locked python tests/system/guest_boot.py

db-up:
    docker compose -f compose.yaml -f compose.dev.yaml --env-file .env up -d --wait postgres

# Retains the PostgreSQL volume.
db-down:
    docker compose -f compose.yaml -f compose.dev.yaml --env-file .env down

# The whole stack in Docker at http://localhost:3000, built from this checkout; CUBEBOT_RUNTIME=kvm|container|off in .env.
up:
    #!/usr/bin/env bash
    set -euo pipefail
    if [ ! -f .env ]; then
        sed "s/^POSTGRES_PASSWORD=.*/POSTGRES_PASSWORD=$(openssl rand -hex 16)/" .env.example > .env
        echo "Created .env with a random POSTGRES_PASSWORD."
    fi
    set -a; source .env; set +a
    if [ -z "${CUBEBOT_RUNTIME:-}" ]; then
        CUBEBOT_RUNTIME=container
        [ "$(uname -s)/$(uname -m)" != Linux/x86_64 ] || [ ! -c /dev/kvm ] || [ ! -c /dev/net/tun ] || CUBEBOT_RUNTIME=kvm
    fi
    if [ "$CUBEBOT_RUNTIME" = kvm ]; then
        if [ ! -f target/guest/bzImage ] || [ ! -f target/guest/root.ext4 ]; then
            echo "Building the guest kernel and root image once (needs make, gcc, flex, bison, bc, libelf-dev)."
            python3 tooling/guest/build.py
        fi
        docker build --tag cubebot-sandbox:local --build-context guest=target/guest --file apps/sandbox/Dockerfile .
    elif [ "$CUBEBOT_RUNTIME" = container ]; then
        docker build --tag cubebot-agent:local --file apps/agent/Dockerfile .
    fi
    CUBEBOT_HOME="$PWD" CUBEBOT_RUNTIME="$CUBEBOT_RUNTIME" CUBEBOT_DEV_CA="$PWD/.cubebot/dev-ca" deploy/cubebot up
    echo "First run: create the administrator with \`just admin\`."

# Stops `just up` and its sandboxes; data stays in Docker volumes (`docker compose down -v` deletes it).
down:
    CUBEBOT_HOME="$PWD" deploy/cubebot down

# Creates the first administrator of the `just up` stack; prompts for the password.
admin login="admin":
    CUBEBOT_HOME="$PWD" deploy/cubebot admin "{{ login }}"

# Always its own disposable PostgreSQL container, never `just db-up`'s or an installation's.
integration:
    uv run --project tooling/quality --locked python tests/system/integration.py

# Owns a disposable local cluster and PostgreSQL; never uses the default kubeconfig.
cluster-admission:
    uv run --project tooling/quality --locked python tests/system/cluster_admission.py

# Uses the shipped agent images and a test-only CSI backend in an owned cluster.
cluster-startup:
    uv run --project tooling/quality --locked python tests/system/cluster_startup.py

# Builds shipped images; requires an external evidence directory and provider key file.
cluster-chat:
    uv run --project tooling/quality --locked python tests/system/cluster_chat.py

# Sandbox scenarios on the kvm backend; x86_64 Linux with /dev/kvm, as cluster-chat.
cluster-sandbox:
    uv run --project tooling/quality --locked python tests/system/cluster_sandbox.py

# A self-signed control CA and a certificate for `names` (default: the docker0 address).
dev-ca directory=".cubebot/dev-ca" names="":
    #!/usr/bin/env bash
    set -euo pipefail
    names="{{ names }}"
    [ -n "$names" ] || names="IP:$(ip -4 -o addr show docker0 | awk '{print $4}' | cut -d/ -f1)"
    deploy/cubebot dev-ca "{{ directory }}" "$names"

# Serves HTTPS on docker0:8443 for Docker sandboxes; needs `just db-up` and the cubebot-sandbox:dev image.
docker-orchestrator: dev-ca
    #!/usr/bin/env bash
    set -euo pipefail
    set -a; source .env; set +a
    bridge=$(ip -4 -o addr show docker0 | awk '{print $4}' | cut -d/ -f1)
    state="$PWD/.cubebot"
    [ -f "$state/credential.key" ] || (umask 077 && head -c 32 /dev/urandom > "$state/credential.key")
    printf '{"image":"cubebot-sandbox:dev","orchestrator_origin":"https://%s:8443","runtime":{"kind":"kvm"},"control_ca_file":"%s","terminal_enabled":true}\n' \
        "$bridge" "$state/dev-ca/ca.crt" > "$state/docker.json"
    CUBEBOT_DATABASE_URL="postgres://cubebot:$POSTGRES_PASSWORD@127.0.0.1:${POSTGRES_PORT:-5432}/cubebot" \
    CUBEBOT_BIND_ADDR="$bridge:8443" \
    CUBEBOT_TLS_CERT_FILE="$state/dev-ca/server.crt" CUBEBOT_TLS_KEY_FILE="$state/dev-ca/server.key" \
    CUBEBOT_CREDENTIAL_KEY_FILE="$state/credential.key" CUBEBOT_DOCKER_CONFIG="$state/docker.json" \
        cargo run --locked -p cubebot-orchestrator

# Against `just docker-orchestrator`: bootstraps once, then creates and wakes an agent.
docker-agent name:
    uv run --project tooling/quality --locked python tests/system/docker_sandbox.py agent "{{ name }}"

# One agent in the container runtime against a scripted provider; Docker only.
container-check:
    uv run --project tooling/quality --locked python tests/system/container_check.py

# One bash tool call through Docker; OPENAI_API_KEY_FILE; CUBEBOT_SANDBOX_RUNTIME=kvm|container (default kvm with /dev/kvm).
docker-sandbox:
    uv run --project tooling/quality --locked python tests/system/docker_sandbox.py

# Three real agents dream, yield and fail through Docker; same needs as docker-sandbox.
docker-dreams:
    uv run --project tooling/quality --locked python tests/system/docker_dreams.py

# The console desktop view on real agents through Docker; docker-sandbox's needs plus Node with Playwright.
docker-desktop:
    uv run --project tooling/quality --locked python tests/system/docker_desktop.py

# The agent's browser tools on real agents through Docker; same needs as docker-sandbox.
docker-browser:
    uv run --project tooling/quality --locked python tests/system/docker_browser.py

# Login handoff on a real agent through Docker; docker-desktop's needs.
docker-login:
    uv run --project tooling/quality --locked python tests/system/docker_login.py

# The vault on two real agents through Docker; docker-desktop's needs.
docker-vault:
    uv run --project tooling/quality --locked python tests/system/docker_vault.py

# Rooms, delegation, messaging, steering, cancellation and lanes on three real agents through Docker; same needs as docker-sandbox.
docker-rooms:
    uv run --project tooling/quality --locked python tests/system/docker_rooms.py

# Shared files on two real agents through Docker; docker-desktop's needs.
docker-files:
    uv run --project tooling/quality --locked python tests/system/docker_files.py

# Routines on two real agents through Docker; docker-desktop's needs.
docker-routines:
    uv run --project tooling/quality --locked python tests/system/docker_routines.py

# Three real agents publish, pin, load, run and dream skills through Docker; same needs as docker-sandbox.
docker-skills:
    uv run --project tooling/quality --locked python tests/system/docker_skills.py
