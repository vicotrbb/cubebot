# Cubebot

A Kubernetes-native home for persistent agents, shared rooms, memories, and skills.

**Status:** a minimal text-chat MVP runs locally: sign-in, agent management, an encrypted OpenAI connection and one permanent conversation per agent, served by the native Rust runtime. A terminal is implemented but disabled by default. See [STATUS](docs/STATUS.md) for everything not yet qualified.

## Run it locally

With Docker (Compose 2.23 or later) and just:

```sh
just up      # builds and starts everything, then waits until it answers
just admin   # once: creates the administrator, prompting for a password
```

Open <http://localhost:3000>, sign in, and add an OpenAI connection in the console. `just down` stops everything and keeps your data in Docker volumes; `docker compose down -v` deletes it. The first `just up` writes `.env` with a random database password; set `CUBEBOT_PORT` there to use another port.

What runs depends on the machine:

- **macOS, Windows, or Linux without KVM:** PostgreSQL, the orchestrator and the console. You can sign in, manage agents and connections, and browse, but an agent cannot wake, because every agent runs in a KVM guest that needs x86_64 Linux.
- **x86_64 Linux with `/dev/kvm`:** the same, plus agents. The first run also builds the guest kernel and root image (`make`, `gcc`, `flex`, `bison`, `bc` and the libelf headers), a sandbox image, and a local control CA. The orchestrator then starts each agent as a Docker container on this host, which needs the Docker socket and is therefore root on this machine. Set `CUBEBOT_AGENTS=0` in `.env` to leave agents off, or `CUBEBOT_AGENTS=1` to fail instead of falling back.

`just up` is for running Cubebot on one machine. It is not a production deployment.

## Start developing

Use Rust through rustup, Bun **1.4.0**, Node **24.18.0**, uv **0.11.26**, and just **1.55.1**. A C/C++ compiler and linker are needed to build the complexity tool. Rust **1.96.0** and Python **3.14.6** are selected by repository files; Python runs development checks only.

```sh
just bootstrap
just check
just smoke
```

`bootstrap` installs locked dependencies and pins Mozilla's complexity analyzer into `.tools/`. `check` runs formatting, linting, the complexity gate, tests, builds and TypeScript checking. `smoke` starts the built services, checks them and stops them.

For development, use two terminals:

```sh
just dev-api
```

```sh
just dev-web
```

Open <http://localhost:3000>. Vite forwards `/api` to the orchestrator at `127.0.0.1:8080`. The orchestrator reads exported `CUBEBOT_BIND_ADDR` and `RUST_LOG`; it does not load `.env` itself.

## Local database

```sh
cp .env.example .env   # choose POSTGRES_PASSWORD
just db-up
export CUBEBOT_DATABASE_URL=...   # your local connection string
CUBEBOT_BOOTSTRAP_LOGIN=admin CUBEBOT_BOOTSTRAP_PASSWORD=... \
  cargo run --locked -p cubebot-orchestrator --bin cubebot-admin -- bootstrap
cargo run --locked -p cubebot-orchestrator
```

`just integration` runs the database, HTTP and OIDC tests against a disposable PostgreSQL container. The [operator and API reference](docs/development/reference.md) covers identity, OIDC, agents, lifecycle controller configuration, runtime recovery and container builds.

## Layout

| Path                | Responsibility                                                         |
| ------------------- | ---------------------------------------------------------------------- |
| `apps/orchestrator` | Installation control plane; identity, PostgreSQL, HTTP and shutdown    |
| `apps/agent`        | Agent runtime; runs its own tools inside its sandbox                   |
| `apps/sandbox`      | `cubebot-sandbox`: in-house KVM VMM and dev process backend (ADR 0084) |
| `apps/web`          | TanStack Start frontend and future PWA                                 |
| `modules/protocol`  | Shared communication contracts                                         |
| `modules/providers` | Bounded native OpenAI text and function-call adapter                   |
| `tooling/quality`   | Complexity gate, regression tests, and Python development environment  |
| `tests/system`      | Smoke checks spanning built application processes                      |
| `deploy`            | Installation packaging; Dockerfiles live with their apps               |
| `site`              | Public landing site: standalone static Astro build, no runtime code    |
| `docs/adr`          | Accepted product and architecture decisions                            |

See [repository layout](docs/development/repository-layout.md) for the module map and dependency rules.

## Read next

- [Manifesto](MANIFESTO.md) and [architecture](ARCHITECTURE.md)
- [Accepted ADRs](docs/adr/README.md)
- [Status](docs/STATUS.md)
- [Software specification](docs/design/software-specification.md) and [qualification plan](docs/design/qualification.md)
- [Contributing](CONTRIBUTING.md)

Licensed under [Apache-2.0](LICENSE).
