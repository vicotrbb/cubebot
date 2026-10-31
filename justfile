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
    bun test apps/web/src/api/client.test.ts apps/web/src/features/desktop.test.ts
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

# The whole stack in Docker at http://localhost:3000 (CUBEBOT_PORT in .env). On x86_64 Linux with /dev/kvm it
# also runs agents, building the guest and sandbox image first; CUBEBOT_AGENTS=0 turns that off, =1 requires it.
up:
    #!/usr/bin/env bash
    set -euo pipefail
    if [ ! -f .env ]; then
        # A random database password; .env stays out of git.
        sed "s/^POSTGRES_PASSWORD=.*/POSTGRES_PASSWORD=$(openssl rand -hex 16)/" .env.example > .env
        echo "Created .env with a random POSTGRES_PASSWORD."
    fi
    set -a; source .env; set +a
    files=(-f compose.yaml)
    platform="$(uname -s)/$(uname -m)"
    if [ "$platform" != Linux/x86_64 ]; then
        missing="agents need x86_64 Linux, this is $platform"
    elif [ ! -c /dev/kvm ] || [ ! -c /dev/net/tun ]; then
        missing="agents need /dev/kvm and /dev/net/tun"
    else
        missing=""
    fi
    agents="${CUBEBOT_AGENTS:-auto}"
    if [ "$agents" = 1 ] && [ -n "$missing" ]; then
        echo "CUBEBOT_AGENTS=1, but $missing." >&2
        exit 1
    fi
    if [ "$agents" = 0 ]; then
        echo "Agents off (CUBEBOT_AGENTS=0)."
    elif [ -n "$missing" ]; then
        echo "Agents off: $missing. The console runs; agents cannot wake."
    else
        if [ ! -f target/guest/bzImage ] || [ ! -f target/guest/root.ext4 ]; then
            echo "Building the guest kernel and root image once (needs make, gcc, flex, bison, bc, libelf-dev)."
            python3 tooling/guest/build.py
        fi
        docker build --tag cubebot-sandbox:local --build-context guest=target/guest --file apps/sandbox/Dockerfile .
        just dev-ca
        export CUBEBOT_DEV_CA="$PWD/.cubebot/dev-ca"
        export CUBEBOT_BRIDGE=$(ip -4 -o addr show docker0 | awk '{print $4}' | cut -d/ -f1)
        export CUBEBOT_DOCKER_GID=$(stat -c %g /var/run/docker.sock)
        files+=(-f compose.agents.yaml)
        echo "Agents on: sandboxes reach the orchestrator at https://$CUBEBOT_BRIDGE:8443."
    fi
    docker compose "${files[@]}" up -d --build --remove-orphans
    origin="http://localhost:${CUBEBOT_PORT:-3000}"
    for _ in $(seq 120); do
        if curl -fsS "$origin/api/health/live" >/dev/null 2>&1; then
            echo "Cubebot is up at $origin. First run: create the administrator with \`just admin\`."
            exit 0
        fi
        sleep 1
    done
    docker compose "${files[@]}" logs --tail 40 orchestrator proxy >&2
    echo "Cubebot did not answer at $origin within 2 minutes; the logs above say why." >&2
    exit 1

# Stops `just up` and the agent sandboxes it started; data stays in Docker volumes (`docker compose down -v` deletes it).
down:
    #!/usr/bin/env bash
    set -euo pipefail
    agents=$(docker ps -q --filter label=cubebot.dev/agent)
    [ -z "$agents" ] || docker stop $agents >/dev/null
    docker compose --env-file .env down

# Creates the first administrator of the `just up` stack; prompts for the password.
admin login="admin":
    #!/usr/bin/env bash
    set -euo pipefail
    set -a; source .env; set +a
    read -rsp "Password for {{ login }}: " password && echo
    CUBEBOT_BOOTSTRAP_LOGIN="{{ login }}" CUBEBOT_BOOTSTRAP_PASSWORD="$password" \
        docker compose --env-file .env run --rm --no-deps -e CUBEBOT_BOOTSTRAP_LOGIN -e CUBEBOT_BOOTSTRAP_PASSWORD \
        --entrypoint cubebot-admin orchestrator bootstrap
    echo "Sign in at http://localhost:${CUBEBOT_PORT:-3000} as {{ login }}."

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

# Three real agents dream, yield and fail through Docker (ADR 0025); same needs as docker-sandbox.
docker-dreams:
    uv run --project tooling/quality --locked python tests/system/docker_dreams.py

# The console desktop view on real agents through Docker (ADRs 0039 and 0085); docker-sandbox's needs plus Node with Playwright.
docker-desktop:
    uv run --project tooling/quality --locked python tests/system/docker_desktop.py

# The agent's browser tools on real agents through Docker (ADRs 0036, 0037 and 0085); same needs as docker-sandbox.
docker-browser:
    uv run --project tooling/quality --locked python tests/system/docker_browser.py

# Three real agents use rooms, delegation, messaging, steering, cancellation and lanes through Docker (ADRs 0005, 0014 to 0016, 0071 to 0073); same needs as docker-sandbox.
docker-rooms:
    uv run --project tooling/quality --locked python tests/system/docker_rooms.py

# Three real agents publish, pin, load, run and dream skills through Docker (ADRs 0055 to 0057); same needs as docker-sandbox.
docker-skills:
    uv run --project tooling/quality --locked python tests/system/docker_skills.py
