# Repository scaffold implementation plan

**Goal:** Establish a buildable Cubebot workspace and reproducible local/CI checks, with cyclomatic complexity at most 15 per function throughout first-party code.

**Architecture:** Follow the four-package Rust layout and dedicated TanStack Start frontend in the accepted architecture. Implement only process liveness and an honest frontend starting page. Runtime, executor, persistence, and Kubernetes behavior remain future work.

**Tech stack:** Rust, Tokio/Axum, TanStack Start/React, Bun/Vite, PostgreSQL development service. Python is development tooling only, for the Mozilla rust-code-analysis gate and its tests.

**Spec:** [Architecture](../../../ARCHITECTURE.md), [software specification](../../design/software-specification.md), and the user's scaffold request.

## Constraints and rulings

- Complexity means cyclomatic complexity per function, maximum 15, including tests and configuration code. No average-based allowance or existing-code baseline.
- Keep lockfiles and toolchain versions. Avoid unused database, Kubernetes, provider, or abstraction dependencies.
- No implementation claims for the native agent loop, authentication, PWA, or cluster lifecycle.
- Work locally in the existing new project; no commits, remotes, publishing, or deployment.
- Execute directly under the user's scaffolding authorization; the architecture discussion already established the design.

## Review focus

- A function at 16 must fail while one at 15 passes in Rust, TS, TSX, JS, and Python.
- A new first-party directory must not silently escape complexity checks.
- Configuration errors must stop the orchestrator; liveness must not claim database or agent readiness.
- A fresh install must generate frontend routes before type checking.
- The built frontend must actually serve under Bun, not merely compile for a different server runtime.

## Tasks

- [x] Create root workspace, version pins, editor/ignore settings, contributor instructions, and development commands.
- [x] Add four Rust packages, a minimal orchestrator with liveness and graceful shutdown, and an HTTP integration test.
- [x] Add TanStack Start frontend with strict TypeScript, formatting and linting, and production Bun server output.
- [x] Add the complexity gate and boundary/discovery regression tests; apply it to all authored code.
- [x] Add CI, development PostgreSQL configuration, container recipes, and setup documentation.
- [x] Run full checks, smoke test actual processes, review the scaffold, and record validation and remaining scope.

## Outcome

Local and fresh-copy bootstrap, quality gate, builds, and process smoke checks passed. The fresh-copy check deliberately omitted the generated route tree. Independent review found a Rust match-arm counting gap in Lizard; regression tests reproduced it, and replacing Lizard with Mozilla rust-code-analysis resolved it. All eight final gate tests pass; the maximum measured source complexity is 13. Container builds remain unverified because Docker ran out of disk space while materializing base images. See [verification](../../development/scaffold-verification.md).
