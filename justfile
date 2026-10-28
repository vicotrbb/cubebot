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
    cargo test -p cubebot-agent --example isolation-probe --locked
    uv run --project tooling/quality --locked python -m unittest discover -s tooling/quality -p 'test_*.py'

build:
    cargo build --workspace --locked
    bun run build
    bun run --cwd site build

# Public landing site; static output in site/dist.
site-dev:
    bun run --cwd site dev

# Building first generates the route tree on a fresh checkout.
check: format-check lint test build
    bun run typecheck

# Starts temporary local processes, checks HTTP behavior, and stops them.
smoke:
    uv run --project tooling/quality --locked python tests/system/smoke.py

db-up:
    docker compose --env-file .env up -d --wait postgres

# Retains the PostgreSQL volume.
db-down:
    docker compose --env-file .env down

# Reuses a running `just db-up` server, else a disposable one; never an application DB.
integration:
    uv run --project tooling/quality --locked python tests/system/integration.py

# Builds owned temporary images and compares Linux isolation mechanisms; not full G1-G3.
container-isolation:
    uv run --project tooling/quality --locked python tests/system/container_isolation.py

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

# Owns a disposable dual-stack cluster for terminal mechanism checks.
cluster-terminal:
    uv run --project tooling/quality --locked python tests/system/cluster_terminal.py
