# Cubebot engineering rules

Read `ARCHITECTURE.md` and the relevant accepted ADRs before changing behavior. `docs/STATUS.md` describes what currently runs and what is unqualified; design documents describe the intended system. Code rules are in `CONTRIBUTING.md`.

- Keep backend, agent runtime, and tool executor in Rust. The frontend uses TanStack Start, React, TypeScript, Bun, and Vite.
- Applications live in `apps/orchestrator`, `apps/agent`, `apps/sandbox`, and `apps/web`. The agent runs its tools in its own process; there is no separate tool runner.
- `apps/sandbox` (`cubebot-sandbox`) owns the VMM (ADR 0084): KVM through `libc` and `nix` ioctls, no VMM crates. It is the only crate that may use `unsafe`, and only for KVM and guest memory. Its `process` backend is development-only and never an isolation claim.
- Keep app-specific behavior inside its app's modules. Only genuinely shared code belongs under `modules/`: communication contracts in `protocol` and provider adapters in `providers`. Shared modules never depend on apps.
- Keep orchestrator rules in cohesive modules; do not add a service or crate per table. Agents never connect directly to PostgreSQL.
- Installation-managed connections default to all eligible agents, with explicit-grant mode available per connection (ADR 0081).
- Every function in a `src/` directory must have cyclomatic complexity <= 15. Tests, scripts and tooling are outside the gate. Refactor responsibilities instead of suppressing the gate or raising its threshold.
- `just check` is the local and CI quality gate. `just smoke` verifies the built HTTP services. New source languages need a supported complexity checker before adding executable code.
- Do not add source exclusions for first-party code. Dependencies, build products, and the exact generated route-tree file are excluded; documentation examples are not executable source.
- Keep lockfiles current. Use meaningful behavior tests, and state which checks actually ran.
- Never treat a liveness response, successful build, or mock provider as proof of agent, security, recovery, or scale qualification.
