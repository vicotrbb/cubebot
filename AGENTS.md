# Cubebot engineering rules

Read `ARCHITECTURE.md` and the relevant accepted ADRs before changing behavior. `README.md` describes what currently runs; design documents describe the intended system.

- Keep backend, agent runtime, and tool executor in Rust. The frontend uses TanStack Start, React, TypeScript, Bun, and Vite.
- Applications live in `apps/orchestrator`, `apps/agent`, and `apps/web`. The agent owns its tool runner as a second executable, not a separate application or crate.
- Keep app-specific behavior inside its app's modules. Only genuinely shared code belongs under `modules/`: communication contracts in `protocol` and provider adapters in `providers`. Shared modules never depend on apps.
- Keep orchestrator rules in cohesive modules; do not add a service or crate per table. Agents never connect directly to PostgreSQL.
- Installation-managed connections default to all eligible agents, with explicit-grant mode available per connection (ADR 0081).
- Every authored function must have cyclomatic complexity <= 15. This includes tests, scripts, and executable configuration. Refactor responsibilities instead of suppressing the gate or raising its threshold.
- `just check` is the local and CI quality gate. `just smoke` verifies the built HTTP services. New source languages need a supported complexity checker before adding executable code.
- Do not add source exclusions for first-party code. Dependencies, build products, and the exact generated route-tree file are excluded; documentation examples are not executable source.
- Keep lockfiles current. Use meaningful behavior tests, and state which checks actually ran.
- Never treat a liveness response, successful build, or mock provider as proof of agent, security, recovery, or scale qualification.
