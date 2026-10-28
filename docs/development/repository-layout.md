# Repository layout

Approved by the user on 2026-10-25. Applications own their implementation. Only communication contracts and model-provider adapters are shared top-level libraries.

```text
cubebot/
├── apps/
│   ├── orchestrator/
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── main.rs
│   │       ├── api/
│   │       ├── identity/
│   │       ├── agents/
│   │       ├── work/
│   │       ├── knowledge/
│   │       ├── connections/
│   │       ├── storage/
│   │       └── operations/
│   ├── agent/
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── main.rs
│   │       ├── engine/
│   │       ├── context/
│   │       ├── memory/
│   │       ├── skills/
│   │       ├── tools/
│   │       └── control/
│   ├── sandbox/
│   │   └── src/
│   └── web/
│       └── src/
│           ├── routes/
│           ├── features/
│           ├── components/
│           └── api/
├── modules/
│   ├── protocol/
│   └── providers/
├── site/
├── tooling/
├── deploy/
├── tests/
└── docs/
```

Each Rust app also has a package-local `src/lib.rs` declaring its internal modules. This supports tests without introducing a separate implementation crate. The directory map shows ownership, not a claim that every domain is implemented. Rust module documentation and frontend directory READMEs preserve the requested structure in version control while stating what remains unimplemented.

## Applications

The orchestrator owns installation-wide state and rules. `api` translates HTTP requests; the other modules own identity/access, agent lifecycle, durable work, knowledge records, installation connections, shared storage, and operations. Database migrations stay with this application. Domain operations that must commit together use one PostgreSQL transaction; module organization does not introduce remote services or independent databases.

The agent owns the native loop and all agent-specific behavior. `engine` coordinates turns and recovery; `context` assembles and compacts working context; `memory` owns extraction and dreaming behavior; `skills` loads installed skill revisions; `tools` discovers and dispatches calls; `control` communicates with the orchestrator. The orchestrator remains authoritative for durable records, grants, credential refresh, and memory/skill revisions.

The agent package has one binary, `cubebot-agent`, and runs its own tools inside its sandbox (ADR 0083).

The sandbox app builds `cubebot-sandbox`, the VMM that runs one agent ([ADR 0084](../adr/0084-in-house-vmm-as-the-agent-sandbox.md)). It owns KVM and guest devices and nothing agent-specific; it is the only crate allowed `unsafe`, for KVM ioctls and guest memory.

The web app owns the console and future PWA. Routes compose features. Feature-specific state and components stay with their feature; reusable interface primitives belong in `components`; API client/types and event transport belong in `api`. The text-chat workspace now implements sign-in/out, authorized agent management, provider configuration and durable conversations.

## Shared libraries and dependencies

- `protocol` defines communication contracts, not PostgreSQL rows or application internals.
- `providers` is reserved for adapters used by agents and the orchestrator's room coordinator. It has no implementation or provider dependencies yet. Adapters never execute tools or own installation credential refresh.
- Shared libraries never depend on applications. Applications do not depend on each other's implementation.
- The agent uses orchestrator protocols rather than importing its database or domain code.
- Add dependencies when the corresponding behavior is implemented. Do not wire empty modules together solely to make the manifests resemble the future dependency graph.

## Development and deployment

`tooling/quality` owns the complexity scanner, regression tests, Python project, lockfile, and version pin. The root `justfile` selects this environment explicitly. The scanner still walks the entire repository and enforces complexity <= 15 per function.

Tests for one package live with that package. `tests/system` owns checks of running applications. Installation packaging belongs in `deploy`; Dockerfiles belong to their applications and use the repository root as their build context. The current local PostgreSQL Compose configuration remains at the root.

`site` is the public landing site: a standalone Astro project with its own lockfile, outside the Bun workspace so it deploys to any static host without the console's dependencies. It contains no Cubebot runtime code. Its scripts are TypeScript and pass the same complexity and lint gates; `.astro` templates are formatted by hand because no Prettier plugin is installed. `PRODUCT.md`, `DESIGN.md` and `.impeccable/` at the root record the product and visual design decisions that the site and the future console share.

Root Cargo and Bun manifests define their respective workspace members explicitly. Rust toolchain and formatting files, lockfiles, editor settings, and repository-wide commands stay at the root for normal tool discovery.
