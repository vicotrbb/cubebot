# OpenBot research

Source: nightly-labs/openbot at cfee2971f29cdb7c16272e60ae912de7fbb30693

- [x] Resolve source and pin revision
- [x] Inventory repository and architecture documentation
- [x] Trace startup, persistence, UI and trust boundaries
- [x] Trace agent runtime, providers and memory
- [x] Trace collaboration, channels and task execution
- [x] Trace tools, integrations, remote clients and automation
- [x] Cross-check specialist findings against source
- [x] Write integrated research reference and source map
- [x] Verify report references and report scope limits

Verification: 291 unique pinned source references and 23 local cross-links passed path/range/anchor checks. Upstream checkout unchanged. No app or provider execution performed.

# Cubebot architecture

Initial production qualification target: 100 registered agents and 25 working simultaneously on meaningful goals, under [ADR 0061](docs/adr/0061-initial-production-qualification-scale.md). [ADR 0062](docs/adr/0062-mixed-organizational-qualification-workloads.md) selects mixed organizational work with specialist collaboration. Exact scenarios, infrastructure, quality, latency, and recovery criteria remain to be specified; no capacity has been validated yet.

- [x] Read the manifesto and requested design skills

Discussion plan, 2026-10-24: After ADR 0077, finish with three product questions covering initial provider support, data retention, and goal completion criteria. Then consolidate the architecture and specification. Propose and justify remaining engineering mechanics in that draft instead of continuing a long sequence of implementation-choice questions. Raise any material contradiction explicitly during review.

The product-question sequence is complete: provider scope in ADR 0078, retention in ADR 0079, and goal completion in ADR 0080. Consolidation and review follow; proposed mechanics do not become accepted decisions without review.

Review amendment on 2026-10-25: ADR 0081 makes installation-managed connections usable by all eligible agents by default, with specific grants selectable per connection. The architecture, specification, related ADRs, and qualification scenarios reflect this accepted change.

- [x] Resolve requirements through focused questions and record ADRs
- [x] Compare consequential design alternatives
- [x] Walk selected designs through normal, concurrent, and failure scenarios
- [x] Assemble the system architecture and software specification
- [ ] Review consistency, scope, and measurable acceptance criteria with the user

Consolidated review draft: [architecture](ARCHITECTURE.md), [software specification](docs/design/software-specification.md), and [qualification and delivery plan](docs/design/qualification.md), dated 2026-10-24. The scenario walkthroughs are design analysis, not executed tests. Implementation proposals, quantitative starting targets, and unresolved qualification gates are labeled in those documents.

Documentation verification on 2026-10-24: 85 Markdown files checked; all 267 checked local links resolve; ADRs 0001 through 0080 are accepted and indexed; the specification's traceability groups cover each ADR exactly once; checked fences and patch-residue checks passed. No application, provider, cluster, or performance tests were performed.

# Repository foundation — 2026-10-25

The user authorized scaffolding after the architecture discussion and required codebase-wide complexity <= 15. The [README](README.md) now separates runnable foundation code from planned agent-system capabilities.

- [x] Initialize local Git repository without commits or a remote
- [x] Create four Rust packages and a dedicated TanStack Start frontend
- [x] Pin toolchains and lock dependencies; add editor and formatting configuration
- [x] Enforce cyclomatic complexity <= 15 across authored code, tests, and tooling
- [x] Add Rust HTTP behavior tests and eight complexity regression tests
- [x] Add local/CI quality commands, container recipes, and optional development PostgreSQL
- [x] Verify actual API and production Bun processes, including shutdown and static assets
- [x] Verify a fresh-copy bootstrap, quality gate, build, and smoke run without the generated route tree
- [x] Review and fix the Rust match-arm complexity bypass in the first candidate analyzer
- [ ] Qualify container builds after resolving local Docker disk exhaustion

Validation: [scaffold verification](docs/development/scaffold-verification.md). No live cluster, provider, agent, database integration, or capacity qualification has been performed. GitHub Actions is configured but has not run on a hosted repository.

# Approved application layout — 2026-10-25

The user replaced the earlier source-layout proposal with exactly three applications: orchestrator, agent, and web. The agent owns its internal tool runner as a second executable. Shared top-level libraries are limited to protocol and providers. See [repository layout](docs/development/repository-layout.md).

- [x] Move the orchestrator to `apps/orchestrator` and declare its requested domain modules
- [x] Consolidate runtime and executor foundations into `apps/agent` with both entry points
- [x] Create `modules/protocol` and `modules/providers`
- [x] Preserve the web UI and add its requested feature, component, and API directories
- [x] Move quality tooling to `tooling/quality` and process smoke checks to `tests/system`
- [x] Update workspace membership, imports, Dockerfiles, CI, commands, and current documentation
- [x] Verify the requested directory tree, binary targets, local quality gate, and process smoke checks
- [x] Review the reorganized source and build paths independently
- [x] Verify fresh-copy install, quality gate, builds, smoke checks, and agent entry-point status
- [ ] Qualify container images after resolving Docker storage exhaustion

The two new agent entry points explicitly exit as unimplemented. No provider, agent loop, or tool execution behavior was added by this reorganization.

Reorganization verification: [layout verification](docs/development/layout-verification.md). Both container builds were attempted again and failed with disk-space errors; no Docker data was pruned.
