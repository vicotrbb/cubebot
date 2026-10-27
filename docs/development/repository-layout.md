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
│   │       ├── bin/
│   │       │   └── tool-runner.rs
│   │       ├── engine/
│   │       ├── context/
│   │       ├── memory/
│   │       ├── skills/
│   │       ├── tools/
│   │       ├── execution/
│   │       └── control/
│   └── web/
│       └── src/
│           ├── routes/
│           ├── features/
│           ├── components/
│           └── api/
├── modules/
│   ├── protocol/
│   └── providers/
├── tooling/
├── deploy/
├── tests/
└── docs/
```

Each Rust app also has a package-local `src/lib.rs` declaring its internal modules. This supports tests without introducing a separate implementation crate. The directory map shows ownership, not a claim that every domain is implemented. Rust module documentation and frontend directory READMEs preserve the requested structure in version control while stating what remains unimplemented.

## Applications

The orchestrator owns installation-wide state and rules. `api` translates HTTP requests; the other modules own identity/access, agent lifecycle, durable work, knowledge records, installation connections, shared storage, and operations. Database migrations stay with this application. Domain operations that must commit together use one PostgreSQL transaction; module organization does not introduce remote services or independent databases.

The agent owns the native loop and all agent-specific behavior. `engine` coordinates turns and recovery; `context` assembles and compacts working context; `memory` owns extraction and dreaming behavior; `skills` loads installed skill revisions; `tools` discovers and dispatches calls; `execution` owns tool-runner behavior; `control` communicates with the orchestrator. The orchestrator remains authoritative for durable records, grants, credential refresh, and memory/skill revisions.

The agent package has two binaries: `cubebot-agent` from `src/main.rs` and `cubebot-tool-runner` from `src/bin/tool-runner.rs`. They belong to one application but run in different containers under [ADR 0033](../adr/0033-separate-runtime-and-tool-containers.md). The tool runner must not inherit runtime identity or provider credentials. The runtime now supports authenticated text-turn grants, native OpenAI inference and durable result recovery. Text-chat lifecycle dispatch launches only the runtime; the pre-existing tool runner remains unavailable and is excluded from that Pod.

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

Root Cargo and Bun manifests define their respective workspace members explicitly. Rust toolchain and formatting files, lockfiles, editor settings, and repository-wide commands stay at the root for normal tool discovery.
