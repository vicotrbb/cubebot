set shell := ["bash", "-eu", "-o", "pipefail", "-c"]

default:
    @just --list

# Install exactly the locked dependency graph.
bootstrap:
    bun install --frozen-lockfile
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
    cargo test --workspace --locked
    cargo test -p cubebot-agent --example isolation-probe --locked
    uv run --project tooling/quality --locked python -m unittest discover -s tooling/quality -p 'test_*.py'

build:
    cargo build --workspace --locked
    bun run build

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

# Uses an isolated disposable local PostgreSQL; never an existing application DB.
integration:
    uv run --project tooling/quality --locked python tests/system/integration.py

# Builds owned temporary images and compares Linux isolation mechanisms; not full G1-G3.
container-isolation:
    uv run --project tooling/quality --locked python tests/system/container_isolation.py

# Owns a disposable local cluster and PostgreSQL; never uses the default kubeconfig.
cluster-admission:
    uv run --project tooling/quality --locked python tests/system/cluster_admission.py

# Uses the shipped agent images and a test-only CSI backend in an owned cluster.
cluster-startup:
    uv run --project tooling/quality --locked python tests/system/cluster_startup.py
