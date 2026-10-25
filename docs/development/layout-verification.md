# Application layout verification — 2026-10-25

This verifies the user-approved repository reorganization, not agent-system functionality.

| Check                         | Result                                                                                                                                   |
| ----------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------- |
| Requested source tree         | Every requested app and module directory exists; legacy `crates/` and `scripts/` directories are removed                                 |
| Cargo metadata                | Two Rust applications and two shared libraries; agent runtime and tool runner are binary targets in one package                          |
| Workspace dependencies        | Shared libraries do not depend on applications; Bun is scoped to `apps/web`                                                              |
| Local `just check`            | Formatting, Clippy, ESLint, Ruff, complexity, Rust tests, eight complexity regression tests, builds, and TypeScript passed               |
| Complexity                    | 31 authored source files, 32 functions/closures, maximum 13 against limit 15                                                             |
| Local `just smoke`            | Configuration rejection, API liveness, graceful shutdown, production Bun SSR, and static assets passed                                   |
| Agent entry points            | Both binaries build and exit with code 1 and an explicit unimplemented message; neither starts agent work or tool execution              |
| Fresh-copy check              | Bootstrap, full check, build, smoke, and both entry-point checks passed in a temporary source copy with the generated route tree omitted |
| Development Compose           | Configuration validation passed; database was not started                                                                                |
| Current documentation         | Checked local links resolve; active source/configuration has no old executable paths                                                     |
| Independent structural review | No blocking defects reported                                                                                                             |
| Orchestrator container build  | Reached dependency compilation, then failed writing build artifacts because Docker storage was full                                      |
| Frontend container build      | Failed materializing a base image because Docker storage was full                                                                        |

Both container failures reported `no space left on device`. Image validation remains incomplete, and no existing Docker data was pruned. Hosted CI was not run. No cluster, provider, persistence workflow, or live agent was exercised.

The frontend UI was preserved. The existing upstream Nitro/Rolldown build warnings remain as documented in the initial scaffold verification. The reorganization adds ownership descriptions and deliberately unimplemented entry points, not new agent capabilities.
