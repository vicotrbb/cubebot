# Scaffold verification — 2026-10-25

Scope: repository foundation, not agent-system qualification.

| Check                       | Observed result                                                                                                                                                  |
| --------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `just bootstrap`            | Locked frontend, Rust, and development dependencies installed; pinned analyzer available                                                                         |
| `just check`                | Formatting, Clippy, ESLint, Ruff, complexity gate, tests, builds, and TypeScript checks passed                                                                   |
| Complexity scan             | 14 authored source files, 30 functions/closures, maximum 13, limit 15                                                                                            |
| Complexity regression tests | Eight tests passed, including 15/16 boundaries for every supported extension, Rust match arms, anonymous TS functions, parser failure, and suppression rejection |
| Rust HTTP test              | Passed: exact liveness body/content type and absent agent API returns 404                                                                                        |
| `just smoke`                | Invalid configuration rejected; API liveness, graceful shutdown, actual production Bun SSR, and linked JS/CSS assets passed                                      |
| Fresh-copy check            | Copied source into a temporary directory, omitted generated routes, then successfully ran bootstrap, check, and smoke                                            |
| Browser                     | Starting page rendered in the in-app browser; no captured console warnings or errors                                                                             |
| Development database config | `docker compose --env-file .env.example config --quiet` passed; database was not started                                                                         |
| Independent review          | Initial Rust match-arm bypass identified, reproduced, fixed, and rechecked; no remaining blocking finding reported                                               |
| Container builds            | Both attempted; Docker failed with `no space left on device` during base-image materialization; builds remain unverified                                         |
| Hosted CI                   | Workflow configured, not executed remotely                                                                                                                       |

The frontend build emits upstream Nitro/Rolldown warnings about React Router's `use client` directives. The pinned Nitro adapter is a prerelease; the observed local SSR, assets, and browser behavior passed. Production deployment remains unqualified.

No live provider, subscription authentication, agent turn, persistence workflow, Kubernetes deployment, security isolation, recovery, or scale test was performed. Runtime and executor crates intentionally contain package documentation only. The optional PostgreSQL configuration and container recipes do not establish installation readiness.

The repository was initialized locally on `main`; no commit, remote, push, release, or deployment was created. Existing Docker data was left intact after the disk-space failure.
