# App and module layout implementation

The user approved three applications: orchestrator, agent, and web. The agent owns its tool runner as a second executable. Only protocol and provider implementations are shared top-level modules. This changes source organization without changing the separate runtime/tool-container decision in ADR 0033.

## Scope

- Move the orchestrator into `apps/orchestrator` and place HTTP routing in `src/api`.
- Declare every requested orchestrator and agent module directory with a concise ownership description. Unimplemented behavior remains explicitly unimplemented.
- Consolidate the runtime and executor foundations into `apps/agent`, with `main.rs` and `bin/tool-runner.rs`. Both entry points fail explicitly until their execution behavior exists.
- Move wire contracts to `modules/protocol`; reserve `modules/providers` as a documented Rust library without inventing provider behavior.
- Add the requested web feature, component, and API directories without changing its UI.
- Move quality tooling and its Python environment configuration to `tooling/quality`; move process smoke checks to `tests/system`.
- Update workspace members, imports, Docker build contexts, CI, commands, and current documentation together. Preserve the complexity cap of 15 and its regression tests.

## Validation

- [x] Requested paths exist and Cargo exposes only the intended applications and shared libraries.
- [x] Formatting, linting, complexity, tests, builds, and type checking pass.
- [x] Existing HTTP and production Bun smoke checks pass after relocation.
- [x] New agent entry points build and explicitly reject execution as unimplemented.
- [x] Fresh-copy install/build/smoke works without stale artifacts or generated routes.
- [x] Review source paths, documentation links, and container build inputs.

Container builds previously failed because Docker storage was full. Do not prune unrelated data; report any remaining build-verification limitation.

## Outcome

The requested layout and Cargo targets are verified. Local and fresh-copy bootstrap, quality checks, builds, smoke checks, and explicit unimplemented agent entry-point behavior passed. The complexity gate checks 31 source files and 32 functions/closures; maximum complexity remains 13. Independent review found no blocking defects. Both Docker builds were attempted but failed because Docker storage was full, so image validation remains incomplete. See [verification](../development/layout-verification.md).
