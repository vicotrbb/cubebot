# Cubebot

A Kubernetes-native home for persistent agents, shared rooms, memories, and skills.

**Status:** a minimal text-chat MVP runs locally: sign-in, agent management, an encrypted OpenAI connection and one permanent conversation per agent, served by the native Rust runtime. A terminal is implemented but disabled by default. See [STATUS](docs/STATUS.md) for everything not yet qualified.

## Install

On Linux or macOS with Docker (Compose 2.23 or later):

```sh
curl -fsSL https://raw.githubusercontent.com/vicotrbb/cubebot/main/deploy/install.sh | sh
```

The installer puts the release's compose files, a `.env` with a random database password, and a `cubebot` command in `~/.cubebot`, pulls the release's images from GHCR, starts them, and prints the address: <http://localhost:3000> unless `CUBEBOT_PORT` is set. The first account created there becomes the administrator; `~/.cubebot/cubebot admin` claims it from the terminal instead, which matters when `CUBEBOT_DOMAIN` makes the console public before you open it. Then add an OpenAI connection in the console. Afterwards:

```sh
~/.cubebot/cubebot up | down | update [version] | logs [-f] | admin [login]
```

`down` keeps your data in Docker volumes; `update` moves to the latest release, or the one named. Set these before running the installer to change what it writes to `~/.cubebot/.env`:

- `CUBEBOT_DOMAIN=cubebot.example.com` serves the console on ports 80 and 443 with a public certificate Caddy obtains; the name must already resolve to this host, and sandboxes reach the orchestrator through it. Without it, the console listens on loopback only and sandboxes reach the orchestrator over a local development CA.
- `CUBEBOT_RUNTIME` is `kvm` by default on x86_64 Linux with `/dev/kvm`, where each agent runs in a KVM guest, and `container` elsewhere, including every Mac, where each agent runs as a plain container ([In-house VMM as the agent sandbox](docs/adr/sandbox.md#in-house-vmm-as-the-agent-sandbox)); `off` runs no agents. The console shows which.
- `CUBEBOT_VERSION` picks a release tag instead of the latest.

Agents are started through the host's Docker socket, which makes the orchestrator root on this machine: install it only where you would give Cubebot that. This is one machine's installation; `deploy/chart` installs on Kubernetes.

## Run from a checkout

```sh
just up      # builds the images from source, runs them as the installer would, and waits until they answer
just admin   # optional: claims the administrator from the terminal instead of the console's setup form
```

`just up` adds `compose.dev.yaml`, which builds the same services `compose.yaml` pulls, and runs them with the same `deploy/cubebot` an installation uses; agents use a locally built sandbox (`kvm`, which also builds the guest kernel and root image and needs `make`, `gcc`, `flex`, `bison`, `bc` and the libelf headers) or agent image (`container`). The first run writes `.env` with a random database password; set `CUBEBOT_PORT`, `CUBEBOT_RUNTIME` or `CUBEBOT_DOMAIN` there. `just down` stops everything and keeps your data; `docker compose -f compose.yaml -f compose.dev.yaml down -v` deletes it.

## Start developing

Use Rust through rustup, Bun **1.4.0**, Node **24.18.0**, uv **0.11.26**, just **1.55.1**, and ShellCheck. A C/C++ compiler and linker are needed to build the complexity tool. Rust **1.96.0** and Python **3.14.6** are selected by repository files; Python runs development checks only.

```sh
just bootstrap
just check
just smoke
```

`bootstrap` installs locked dependencies and pins Mozilla's complexity analyzer into `.tools/`. `check` runs formatting, linting (ShellCheck for the installer), the complexity gate, tests, builds and TypeScript checking. `smoke` starts the built services, checks them and stops them.

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

| Path                | Responsibility                                                                                                                                           |
| ------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `apps/orchestrator` | Installation control plane; identity, PostgreSQL, HTTP and shutdown                                                                                      |
| `apps/agent`        | Agent runtime; runs its own tools inside its sandbox                                                                                                     |
| `apps/sandbox`      | `cubebot-sandbox`: in-house KVM VMM and dev process backend ([In-house VMM as the agent sandbox](docs/adr/sandbox.md#in-house-vmm-as-the-agent-sandbox)) |
| `apps/web`          | TanStack Start frontend and future PWA                                                                                                                   |
| `modules/protocol`  | Shared communication contracts                                                                                                                           |
| `modules/providers` | Bounded native OpenAI text and function-call adapter                                                                                                     |
| `tooling/quality`   | Complexity gate, regression tests, and Python development environment                                                                                    |
| `tests/system`      | Smoke checks spanning built application processes                                                                                                        |
| `deploy`            | Installation packaging; Dockerfiles live with their apps                                                                                                 |
| `site`              | Public landing site: standalone static Astro build, no runtime code                                                                                      |
| `docs/adr`          | Accepted product and architecture decisions                                                                                                              |

See [repository layout](docs/development/repository-layout.md) for the module map and dependency rules.

## Read next

- [Manifesto](MANIFESTO.md) and [architecture](ARCHITECTURE.md)
- [Accepted decisions](docs/adr/README.md)
- [Status](docs/STATUS.md)
- [Software specification](docs/design/software-specification.md) and [qualification plan](docs/design/qualification.md)
- [Contributing](CONTRIBUTING.md)

Licensed under [Apache-2.0](LICENSE).
