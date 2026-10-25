# ADR 0077: Coordinated maintenance upgrades

Status: Accepted

Date: 2026-10-24

Decision: Upgrade Cubebot through a coordinated maintenance window. Stop admitting new work, bring affected execution and writers to safe checkpoints, upgrade platform software and agent runtimes, then resume eligible work after recovery checks. The user selected this model on 2026-10-24. Continuous operation with mixed agent-runtime versions is outside the initial upgrade contract.

Usage: An administrator starts an upgrade. Cubebot reports preparation and draining, identifies any operation that cannot settle, applies the release's migrations and workload changes once safe, verifies recovery, and reopens admission. Agents continue from durable progress where possible. Live browser sessions and arbitrary processes may need to restart.

Ownership: A durable maintenance workflow coordinates admission, checkpoints, lifecycle changes, and recovery. Runtime and tool modules account for active work and filesystem writers. Persistence owns migrations and data compatibility. Deployment tooling applies release artifacts and must be able to report or recover a failed upgrade when the ordinary application process is unavailable. Exact packaging and command interfaces remain engineering choices.

Reuse the maintenance and writer-control contract from ADR 0065. Account for dreaming, extraction, tool processes, browser activity, uploads, skill publication, background workers, and configuration changes as applicable. Disconnected agents must acknowledge checkpoints or be safely fenced under their finite authority. Missing heartbeats do not establish a clean checkpoint. Drain deadlines expose blockers and defer or fail the upgrade; forced termination is not proof of preserved process state.

New requests receive explicit maintenance behavior. Never acknowledge acceptance unless the work is durably retained across the upgrade. If database migration prevents safe acceptance, return a retryable maintenance response and retain local drafts under ADR 0053. Exact queuing and read-only availability during each phase remain open.

Track upgrade progress durably outside assumptions about a particular process surviving. Restart must reconcile which release, schema, and agent versions are present before enabling execution. Temporary version differences during replacement do not imply supported mixed-version work: incompatible agents remain blocked from new assignments until replaced or recovered. An interrupted migration or failed rollout must not silently reopen the installation.

Resume rechecks execution ownership, pending tool outcomes, permissions, cancellations, and configured budgets. Keep explicitly cancelled, budget-paused, or otherwise blocked work in its appropriate state. A checkpoint cannot justify replaying an external action whose outcome is uncertain. Agent identity, recorded progress, history, and workspace files survive subject to the durable storage contract; in-memory state is not promised to survive.

Each release needs a documented migration and recovery path, with a coordinated recoverable backup before changes that require one. Do not assume deploying an older image reverses schema or data changes. Recovery may require restoration of a compatible database, shared files, workspaces, and keys under ADR 0064. The release must state whether rollback or forward repair is supported and how unresolved external effects are handled.

Alternatives considered: Upgrade the orchestrator while compatible older runtimes keep working, then replace agents gradually. The user selected maintenance upgrades, accepting interruption and avoiding a requirement for continuous mixed-version execution in the initial architecture.

Open details: Release packaging; migration tooling; exact phase states and deadlines; maintenance ingress; backup prerequisites; version compatibility checks; upgrade diagnostics; and rollback versus forward-repair procedures.

Validation scenarios: Drain active and delegated work; report an unsafe tool operation; account for a disconnected writer; preserve drafts and accepted work; recover an interrupted upgrade or migration; block incompatible agents; verify storage and authentication before resuming; retain cancellation and pause state; and resume without duplicating external actions. Implementation and upgrade qualification remain pending.
