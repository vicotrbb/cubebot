# Cubebot implementation progress

Plan: [end-to-end implementation](../superpowers/plans/2026-10-25-cubebot.md).

## 2026-10-25

- Inspected manifesto, architecture, specification, qualification, ADR index and persistence/identity ADRs. Baseline contains only liveness/frontend foundation and intentionally unimplemented agent binaries.
- Working branch: `codex/end-to-end`. No deployment, publication or external infrastructure changes authorized.
- Ruling: execute in the current clean checkout on a feature branch, avoiding duplicate build/dependency storage on a host with about 11 GiB free. Existing unrelated Docker services are out of scope.
- Ruling: existing approved architecture is the design baseline; proceed with routine implementation choices under the user's explicit autonomy instruction rather than introducing another design approval gate.
- Task 1 in progress. Remaining milestones pending. No runtime, security, recovery or scale qualification claimed.

### First persistence/identity subtask

Implemented PostgreSQL 18 integration, embedded SQLx migration tracking, bounded pool startup and configured-database readiness. Added local one-time operator bootstrap, Argon2id passwords, opaque one-day sessions with only SHA-256 digests retained, authenticated account inspection, administrator account creation, logout and suspension with live activity checks. Bootstrap, account creation and suspension record identity audit entries. No public registration/bootstrap endpoint exists.

Verification on 2026-10-25:

- Baseline `cargo test --workspace --locked`: passed before changes.
- Red checks: persistence test failed for missing database module; identity test failed for missing Identity API; HTTP test failed for missing database router.
- `just integration`: passed against a disposable PostgreSQL 18 container. Six integration/unit HTTP cases passed, plus CLI bootstrap/login/server-restart/logout workflow. Each database test creates its own database inside that disposable instance.
- `just check`: passed formatting, linting, complexity, Rust/Python tests, builds and TypeScript checks. Normal Rust runs explicitly ignore opt-in database tests; CI now runs `just integration` separately.
- `just smoke`: passed actual orchestrator liveness/shutdown and Bun SSR/assets.
- `just complexity`: 44 source files, 97 functions, maximum 14, limit 15.
- Frontend build retains the existing upstream TanStack `use client` warnings. No new frontend behavior was added.
- Runner correction: PostgreSQL's temporary initialization server accepts Unix sockets before the final TCP listener. Readiness now probes TCP, matching the actual application connection.

Milestone 1 remains in progress. Invitations, local teams, scoped grants, OIDC, authentication abuse controls and the complete identity acceptance workflow remain pending. No agent, tool, cluster, backup, live-provider or scale qualification. Existing Docker workloads were untouched; only disposable Cubebot test containers were created and removed. Container image builds were not repeated in this subtask.

Implementation references: [SQLx embedded migrations](https://docs.rs/sqlx/0.8.6/sqlx/macro.migrate.html) and [RustCrypto Argon2](https://docs.rs/argon2/0.5.3/argon2/). SQLx 0.8.6 was selected with PostgreSQL, migration, UUID and Tokio/Rustls features; no generic persistence abstraction was introduced.

Independent review found an unbounded SQLx migration wait after pool acquisition. Added server-side five-second statement deadlines and a ten-second overall migration deadline on a detached connection, so advisory locks cannot return to the application pool. A real table-lock regression failed at its eight-second outer guard before the fix and passed after it; migration replay after releasing the blocker also passed. Reviewer confirmed the fix and reported no remaining findings in this subtask.

Final subtask verification: `just integration` passed seven Rust cases and the CLI/HTTP process restart workflow; `just check` passed with 44 source files, 100 functions, maximum complexity 14; `just smoke` passed before the deadline-only change and is rerun for the checkpoint. Milestone 1 remains incomplete; the next subtask adds durable command identities, invitations, teams and scoped access before OIDC and authentication abuse controls.
