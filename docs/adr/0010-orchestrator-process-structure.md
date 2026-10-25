# ADR 0010: One orchestrator process

Status: Accepted

Date: 2026-10-23

The orchestrator mediates durable state access under [ADR 0009](0009-agent-state-access.md) and manages agent pod lifecycle under [ADR 0007](0007-agent-idle-lifecycle.md). Deployment must place application-facing operations and Kubernetes reconciliation into explicit module ownership.

Decision: Run application handling and Kubernetes lifecycle management in one Rust orchestrator process. The user selected this model on 2026-10-23. Do not deploy a separate Kubernetes controller process for the initial architecture.

The process contains modules for client and agent interfaces, authorization, rooms and messaging, scheduling, memory persistence, and Kubernetes lifecycle reconciliation. These modules retain their own rules and interfaces. One process does not require one undifferentiated implementation, one thread, or one global lock.

Usage: A message is durably accepted, scheduling determines that its agent needs execution, and the lifecycle module reconciles the desired running state with Kubernetes. When the agent pod becomes ready and authenticates, it can receive its work. Failed or delayed pod creation does not discard the accepted message.

Ownership: Domain modules own permissions and durable state transitions. The Kubernetes lifecycle module owns translation of desired agent state into cluster operations and observed status. Its implementation contains Kubernetes-specific details, so messaging and memory callers do not need to construct pod specifications or understand Kubernetes request ordering. Network and reconciliation work must not block unrelated request handling.

Rationale: The chosen deployment keeps installation and operation simple, with one orchestrator artifact and no internal controller network protocol. Module interfaces provide implementation separation without an additional deployed process.

Alternative considered: A separate controller would isolate Kubernetes management credentials and allow independent restarts and scaling, at the cost of another deployment and coordination between processes. The user selected the combined process.

Consequences: The application-facing process also holds the Kubernetes permissions required for agent lifecycle operations. These permissions must be scoped to the resources Cubebot manages. A process failure affects request handling and reconciliation together. Durable intent and restart reconciliation remain necessary; in-memory coordination cannot be the sole record of accepted work.

Scope: This decision selects the backend process structure. [ADR 0011](0011-orchestrator-availability.md) subsequently selects one active instance with automatic restart and accepts recovery downtime. Agent runtimes remain separate pods, and PostgreSQL remains the authoritative structured store. [ADR 0069](0069-dedicated-tanstack-start-frontend.md) adds a dedicated TanStack Start frontend server using Bun and Vite; backend domain rules and Kubernetes reconciliation remain in the Rust orchestrator.

[ADR 0074](0074-postgresql-owned-agent-definitions.md) makes PostgreSQL agent definitions and lifecycle intent authoritative. The Kubernetes lifecycle module reconciles cluster resources from that intent and records observed status; no Agent custom resource is required initially.

Open details: Recovery-time targets; reconciliation mechanics; Kubernetes workload objects and permissions; internal concurrency; graceful shutdown; deployment and upgrade behavior; and observability.

Validation scenarios: Accept work while pod creation is delayed; reconcile an agent after process restart; retry failed cluster operations without creating competing executors; keep ordinary requests responsive while reconciliation waits; and restrict lifecycle operations to authorized managed resources.
