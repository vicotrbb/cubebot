# Cubebot

A Kubernetes-native home for persistent agents, shared rooms, memories, and skills.

**Status:** a minimal text-chat MVP runs locally: sign-in, agent management, an encrypted OpenAI connection and one permanent conversation per agent, served by the native Rust runtime. A terminal is implemented but disabled by default. See [STATUS](docs/STATUS.md) for everything not yet qualified.

## Install

On Linux or macOS with Docker (Compose 2.23 or later):

```sh
curl -fsSL https://cubebot.io/install.sh | bash
```

The installer checks Docker, finds the latest release and asks four questions, each with its default selected (arrows, `j`/`k` or a digit, then Enter):

- **Where agents run.** `kvm` runs each agent in its own KVM guest and needs x86_64 Linux with `/dev/kvm` and `/dev/net/tun`; it is the default there and marked unavailable elsewhere. `container` runs each agent as a plain container, a weaker boundary that works everywhere, including every Mac ([In-house VMM as the agent sandbox](docs/adr/sandbox.md#in-house-vmm-as-the-agent-sandbox)). `off` runs no agents. The console shows which.
- **How the console is reached.** This machine, on loopback at <http://localhost:3000> (`CUBEBOT_PORT` changes the port), where sandboxes reach the orchestrator over a local development CA; or a public domain, served on ports 80 and 443 with a certificate Caddy obtains. The domain must already resolve to this machine, and sandboxes reach the orchestrator through it.
- **Install directory**, `~/.cubebot` by default: the release's compose files, a `.env` with a random database password, and the `cubebot` command. Running the installer again updates the installation there and never replaces its `.env`.
- **Administrator.** Create it now in the terminal (login and password), or let the first account created at the console become the administrator. With a public domain, now is preselected, since otherwise whoever opens the console first claims it.

It then shows a summary, asks to install, pulls the release's images from GHCR, starts them and prints the address. Add a provider key from the gear beside your name (AI providers) to run agents. Every question has a variable, `CUBEBOT_RUNTIME`, `CUBEBOT_ACCESS` (`local` or `domain`, with `CUBEBOT_DOMAIN`), `CUBEBOT_HOME` and `CUBEBOT_ADMIN` (`now` or `console`, with `CUBEBOT_BOOTSTRAP_LOGIN` and `CUBEBOT_BOOTSTRAP_PASSWORD`); a question whose variable is set is not asked, and `CUBEBOT_YES=1` or no terminal takes every default. `CUBEBOT_VERSION` picks a release tag instead of the latest; `NO_COLOR` turns colors off. Afterwards:

```sh
~/.cubebot/cubebot up | down | update [version] | logs [-f] | admin [login] | uninstall [--yes]
```

`down` keeps your data in Docker volumes; `update` moves to the latest release, or the one named; `admin` creates the administrator from the terminal while nobody has. `uninstall` asks you to type `uninstall` (or takes `--yes`), then removes every agent the installation created with its workspace, the containers, the volumes with all data, the Cubebot images of every release it ran and `~/.cubebot`, and prints what it removed; other installations on the machine are left alone.

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
| `modules/providers` | Native OpenAI, Anthropic and OpenRouter adapters                                                                                                         |
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
