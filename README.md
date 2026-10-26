# Cubebot

A Kubernetes-native home for persistent agents, shared rooms, memories, and skills.

**Status: first persistence and identity slice in progress.** The Rust orchestrator supports PostgreSQL migrations, operator-only administrator bootstrap, built-in account login, expiring sessions, logout, administrator account creation and suspension. The TanStack Start frontend still serves a starting page. Invitations, teams, scoped grants, OIDC, agent execution, rooms, memory, integrations, PWA installation and Kubernetes reconciliation are not implemented yet.

## Start developing

Use Rust through rustup, Bun **1.4.0**, Node **24.18.0**, uv **0.11.26**, and just **1.55.1**. A C/C++ compiler and linker are needed to build the syntax-aware complexity tool. Rust **1.96.0** and Python **3.14.6** are selected by repository files; uv can provision Python. The Python project and version pin live in `tooling/quality`. Python runs development checks only, not any Cubebot service.

```sh
just bootstrap
just check
just smoke
```

`bootstrap` installs locked Rust, frontend, and development dependencies, and pins Mozilla's complexity analyzer into the ignored `.tools/` directory. `check` runs formatting, linting, complexity enforcement, tests, builds, and TypeScript checking. The frontend build generates its route tree before type checking. `smoke` requires built artifacts and starts temporary local processes, checks them, and stops them.

For development, use two terminals:

```sh
just dev-api
```

```sh
just dev-web
```

Open <http://localhost:3000>. Vite forwards `/api` to the orchestrator at `127.0.0.1:8080`. `GET /api/health/live` returns `{"status":"alive"}`; this is process liveness, not database readiness or a functioning agent engine. The production frontend is a separate server; production `/api` routing will belong to the installation ingress.

The orchestrator accepts exported `CUBEBOT_BIND_ADDR` and `RUST_LOG` variables. It defaults to `127.0.0.1:8080` and `info`; it does not load `.env` itself. Invalid values stop startup. SIGINT and SIGTERM request graceful shutdown.

## Layout

| Path                | Responsibility                                                        |
| ------------------- | --------------------------------------------------------------------- |
| `apps/orchestrator` | Installation control plane; identity, PostgreSQL, HTTP and shutdown   |
| `apps/agent`        | Agent runtime and internal tool runner in one Rust package            |
| `apps/web`          | TanStack Start frontend and future PWA                                |
| `modules/protocol`  | Shared communication contracts                                        |
| `modules/providers` | Shared provider adapter foundation; no providers implemented yet      |
| `tooling/quality`   | Complexity gate, regression tests, and Python development environment |
| `tests/system`      | Smoke checks spanning built application processes                     |
| `deploy`            | Installation packaging; Dockerfiles live with their apps              |
| `docs/adr`          | Accepted product and architecture decisions                           |

See [repository layout](docs/development/repository-layout.md) for the complete module map and dependency rules. The agent package builds `cubebot-agent` and `cubebot-tool-runner`; both explicitly exit with an unimplemented message until their execution behavior exists. The tool runner belongs to the agent app and will run in its separate tool container.

The backend remains a modular monolith. PostgreSQL uses SQLx with embedded versioned migrations. Kubernetes, provider and tool dependencies will be added with their first working feature.

## Quality rules

**Cyclomatic complexity must be <= 15 per authored function, including tests and tooling.** This is a hard maximum, not a codebase average. `just complexity` scans the repository, including newly added directories. Rust `match` arms, anonymous functions, and the inclusive 15/16 boundary have regression coverage.

See [the quality policy](docs/development/quality.md) for exact tools, scope, exclusions, and limitations. `just format` applies formatting. GitHub Actions runs the same `just check` and `just smoke` commands and separately builds both containers. Hosting the workflow does not enable branch protection; configure required checks when a remote repository is created.

## Optional local database

The liveness-only process still runs without a database. To run the identity API with PostgreSQL:

```sh
cp .env.example .env
# Choose your local POSTGRES_PASSWORD in .env.
just db-up
just db-down
```

Compose binds PostgreSQL to loopback and retains its named volume when stopped. The example credentials are for local development. The orchestrator applies embedded migrations when `CUBEBOT_DATABASE_URL` is exported. Invalid configured databases stop startup. The development image follows PostgreSQL major 18; production image digests and pgvector remain unqualified.

### Identity development workflow

Export `CUBEBOT_DATABASE_URL` with your local PostgreSQL connection string. Create the initial administrator through the local operator binary, using exported `CUBEBOT_BOOTSTRAP_LOGIN` and `CUBEBOT_BOOTSTRAP_PASSWORD`, then remove those bootstrap variables from your shell:

```sh
cargo run --locked -p cubebot-orchestrator --bin cubebot-admin -- bootstrap
cargo run --locked -p cubebot-orchestrator
```

Bootstrap is one-time and serialized in PostgreSQL. Passwords must contain 12–1024 bytes and logins use 3–64 lowercase ASCII letters, digits, dots, underscores or hyphens, starting with a letter or digit. Passwords use Argon2id. Sessions last one day and persist only SHA-256 token digests. Send the returned opaque token as `Authorization: Bearer <token>` over HTTPS outside loopback development. No public bootstrap or registration endpoint exists.

The current identity API has `POST /api/identity/login`, `GET /api/identity/me`, `POST /api/identity/logout`, administrator-only `POST /api/identity/accounts`, and administrator-only `POST /api/identity/suspend`. Login and account creation accept `login` and `password`; creation also accepts `administrator`, default false. Suspension accepts account `id`. Logout removes the session, and suspension blocks subsequent authentication and removes the account's sessions. These endpoints are an initial development slice; scoped access, invitation/OIDC flows and authentication abuse controls remain pending.

`GET /api/health/ready` checks the configured database and exists only in database-enabled mode. It does not attest to agent, Kubernetes or provider readiness.

```sh
just integration
```

This builds the orchestrator/operator binaries, starts an isolated disposable PostgreSQL 18 container with a generated password, runs real database/HTTP tests and a process restart workflow, then removes that container and its test data. It never uses the database in `.env` or another application's database. Ordinary `cargo test` explicitly reports the database cases as ignored; `just integration` runs them and fails if Docker or PostgreSQL is unavailable.

## Builds and containers

```sh
cargo build --release --workspace --locked
bun run build
bun run --cwd apps/web start
```

The frontend server listens on port 3000 by default. Set `HOST=127.0.0.1` for local-only production preview. Its [documented Bun deployment adapter](https://tanstack.com/start/latest/docs/framework/react/guide/hosting#bun) is Nitro, pinned to `3.0.260903-beta`. This is a prerelease dependency; Bun SSR and static assets are smoke-tested, while production hosting still requires qualification. The current build emits upstream bundler warnings about React Router's `use client` directives; the local browser check found no console errors or warnings.

```sh
docker build -t cubebot-orchestrator:dev -f apps/orchestrator/Dockerfile .
docker build -t cubebot-web:dev -f apps/web/Dockerfile .
```

Container recipes run as non-root users. They are development build foundations, not a qualified installation. Helm packaging, agent pod construction, network enforcement, credential isolation, and fencing belong to subsequent implementation milestones.

Container verification was retried after the layout change on 2026-10-25. Both builds remained blocked by Docker's `no space left on device` error: the orchestrator during dependency compilation and the frontend while materializing its base image. No existing Docker data was pruned. See [layout verification](docs/development/layout-verification.md) for the successful local and fresh-copy checks; image builds remain unqualified.

## Design and next milestone

- [Manifesto](MANIFESTO.md)
- [Architecture](ARCHITECTURE.md)
- [Accepted ADRs](docs/adr/README.md)
- [Software specification](docs/design/software-specification.md)
- [Qualification plan](docs/design/qualification.md)
- [Implementation milestones](docs/superpowers/plans/2026-10-25-cubebot.md)
- [Implementation evidence](docs/development/implementation-progress.md)

Next, implement a complete path from creating an agent and recording its assignment to native execution, streamed results, and restart-safe state. The scaffold does not resolve the security and recovery qualification gates described in those documents.
