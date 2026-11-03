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
    bun test apps/web/src/api/client.test.ts apps/web/src/features/desktop/desktop.test.ts apps/web/src/features/platform/sandbox.test.ts
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

# Lints the chart and renders each runtime and database mode with deploy/chart/ci/values.yaml.
chart:
    helm lint --strict deploy/chart --values deploy/chart/ci/values.yaml
    helm template cubebot deploy/chart --values deploy/chart/ci/values.yaml > /dev/null
    helm template cubebot deploy/chart --values deploy/chart/ci/values.yaml --set sandbox.runtime=kvm,controlCA=pem > /dev/null
    helm template cubebot deploy/chart --values deploy/chart/ci/values.yaml --set externalDatabase.existingSecret=database > /dev/null

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

# Installs a build of this checkout with deploy/install.sh from a local registry, signs in and wakes one container agent; Docker only.
install-check:
    uv run --project tooling/quality --locked python tests/system/install_check.py

# Starts temporary local processes, checks HTTP behavior, and stops them.
smoke:
    uv run --project tooling/quality --locked python tests/system/smoke.py

# Builds the guest into target/guest and runs its acceptance in a network namespace (sudo); skips off x86_64 Linux.
guest-boot:
    uv run --project tooling/quality --locked python tests/system/guest_boot.py

db-up:
    docker compose -f compose.yaml -f compose.dev.yaml --env-file .env up -d --wait postgres

# Retains the PostgreSQL volume.
db-down:
    docker compose -f compose.yaml -f compose.dev.yaml --env-file .env down

# The whole stack in Docker at http://localhost:3000 (CUBEBOT_PORT in .env), built from this checkout and run by
# deploy/cubebot as an installed release runs. Agents run as KVM guests on x86_64 Linux with /dev/kvm, building the
# guest first, else as containers; CUBEBOT_RUNTIME=kvm|container|off in .env chooses.
up:
    #!/usr/bin/env bash
    set -euo pipefail
    if [ ! -f .env ]; then
        # A random database password; .env stays out of git.
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

# Stops `just up` and the agent sandboxes it started; data stays in Docker volumes (`docker compose down -v` deletes it).
down:
    CUBEBOT_HOME="$PWD" deploy/cubebot down

# Creates the first administrator of the `just up` stack; prompts for the password.
admin login="admin":
    CUBEBOT_HOME="$PWD" deploy/cubebot admin "{{ login }}"

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

# docs/adr/sandbox.md#agent-centric-execution-in-a-sandbox-boundary sandbox scenarios on the kvm backend; x86_64 Linux with /dev/kvm, as cluster-chat.
cluster-sandbox:
    uv run --project tooling/quality --locked python tests/system/cluster_sandbox.py

# A self-signed control CA and a certificate for `names` (default: the docker0 address); kept once made, remove the directory to reissue.
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

# Against `just docker-orchestrator`: bootstraps the administrator and OpenAI provider once, then creates and wakes an agent.
docker-agent name:
    uv run --project tooling/quality --locked python tests/system/docker_sandbox.py agent "{{ name }}"

# One agent in the container runtime against a scripted provider: a bash call, and the desktop through the host channel; Docker only.
container-check:
    uv run --project tooling/quality --locked python tests/system/container_check.py

# Boots one agent through Docker and runs one bash tool call; OPENAI_API_KEY_FILE, and for CUBEBOT_SANDBOX_RUNTIME=kvm (the default with /dev/kvm) x86_64 Linux.
docker-sandbox:
    uv run --project tooling/quality --locked python tests/system/docker_sandbox.py

# The docker-* suites take CUBEBOT_SANDBOX_RUNTIME=kvm|container (default: kvm where /dev/kvm exists).
# Three real agents dream, yield and fail through Docker (docs/adr/memory.md#idle-only-dreaming-with-visible-status-and-results); same needs as docker-sandbox.
docker-dreams:
    uv run --project tooling/quality --locked python tests/system/docker_dreams.py

# The console desktop view on real agents through Docker (docs/adr/sandbox.md#live-browser-viewing-and-explicit-human-takeover and docs/adr/sandbox.md#desktop-as-a-vmm-device); docker-sandbox's needs plus Node with Playwright.
docker-desktop:
    uv run --project tooling/quality --locked python tests/system/docker_desktop.py

# The agent's browser tools on real agents through Docker (docs/adr/sandbox.md#structured-browser-tools-with-visual-interaction, docs/adr/sandbox.md#main-browser-session-with-separate-task-sessions and docs/adr/sandbox.md#desktop-as-a-vmm-device); same needs as docker-sandbox.
docker-browser:
    uv run --project tooling/quality --locked python tests/system/docker_browser.py

# Login handoff on a real agent through Docker (docs/adr/sandbox.md#live-browser-viewing-and-explicit-human-takeover and docs/adr/sandbox.md#desktop-as-a-vmm-device); docker-desktop's needs.
docker-login:
    uv run --project tooling/quality --locked python tests/system/docker_login.py

# The vault on two real agents through Docker, from every scope and person's side (docs/adr/identity.md#a-vault-the-desktop-arbiter-fills); docker-desktop's needs.
docker-vault:
    uv run --project tooling/quality --locked python tests/system/docker_vault.py

# Three real agents use rooms, delegation, messaging, steering, cancellation and lanes through Docker (docs/adr/agents.md#agent-execution-lanes, docs/adr/rooms.md#direct-mentions-and-a-tool-free-room-coordinator, docs/adr/rooms.md#independent-assignments-for-multiple-room-mentions, docs/adr/rooms.md#explicit-agent-messaging-and-delegation, docs/adr/rooms.md#queued-messages-with-explicit-steering, docs/adr/rooms.md#cancel-selected-work-and-its-descendants and docs/adr/rooms.md#contain-delegated-failures-and-let-the-parent-adapt); same needs as docker-sandbox.
docker-rooms:
    uv run --project tooling/quality --locked python tests/system/docker_rooms.py

# Shared files on two real agents through Docker, from every scope and person's side (docs/adr/memory.md#orchestrator-managed-file-storage-on-a-persistent-volume and docs/adr/memory.md#native-cubebot-storage-api); docker-desktop's needs.
docker-files:
    uv run --project tooling/quality --locked python tests/system/docker_files.py

# Routines on two real agents through Docker, from every scope and person's side (docs/adr/agents.md#routines); docker-desktop's needs.
docker-routines:
    uv run --project tooling/quality --locked python tests/system/docker_routines.py

# Three real agents publish, pin, load, run and dream skills through Docker (docs/adr/memory.md#reusable-skills-created-by-humans-agents-and-dreaming, docs/adr/memory.md#shared-installation-skill-library-by-default and docs/adr/memory.md#shared-skill-revisions-by-any-agent-or-user); same needs as docker-sandbox.
docker-skills:
    uv run --project tooling/quality --locked python tests/system/docker_skills.py
