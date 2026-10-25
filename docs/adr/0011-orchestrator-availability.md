# ADR 0011: One active orchestrator with automatic restart

Status: Accepted

Date: 2026-10-23

[ADR 0010](0010-orchestrator-process-structure.md) combines application handling and Kubernetes lifecycle management in one Rust process. The installation needs an explicit availability model for failure and upgrades.

Decision: Run one active orchestrator instance with automatic restart. The user selected this model on 2026-10-23 and accepted recovery downtime for the initial release. Multiple active replicas and standby failover are outside the initial deployment design.

During an orchestrator outage, its client interfaces and agent state operations are unavailable. New work cannot be acknowledged as durably accepted through the unavailable orchestrator. Already accepted work remains in PostgreSQL. [ADR 0012](0012-agent-disconnection-behavior.md) permits running agents to continue their current turns within existing authorization and limits, with durable local journaling and reconnection reconciliation.

On restart, the orchestrator recovers durable intent, observes existing agent execution, and reconciles pending operations before resuming coordination. It must not assume every previously running turn failed or blindly repeat actions whose outcome is uncertain. Recovery must distinguish accepted work, observed completion, and unresolved outcomes.

Rationale: A single active instance keeps deployment and operation simple while retaining durable coordination and explicit restart recovery. Capacity will be measured against representative workloads rather than inferred from replica count.

Alternative considered: Multiple replicas could serve requests through an instance failure but require coordinated scheduling and lifecycle ownership, stale-owner protection, and connection recovery across replicas. The user chose restart recovery with downtime instead.

Consequences: Orchestrator failure and upgrades interrupt coordination and console operations until recovery succeeds. Automatic restart does not guarantee a particular recovery time or success when configuration, PostgreSQL, or cluster capacity is unavailable. PostgreSQL availability and backup policies remain separate decisions.

One active instance is an operational invariant, not merely a desired replica count. Restart, upgrade, and network-partition handling must prevent an old instance from acting concurrently with its replacement. The mechanism will be specified with execution ownership and recovery; this decision does not require a general multi-replica scheduling implementation.

[ADR 0061](0061-initial-production-qualification-scale.md) sets the initial qualification target at 100 registered agents and 25 working simultaneously on meaningful goals. This target must be measured with the selected single-active-orchestrator design; it is not a current capacity claim.

[ADR 0077](0077-coordinated-maintenance-upgrades.md) selects coordinated maintenance upgrades with checkpointing and recovery checks before resuming eligible execution.

Open details: Recovery-time targets; readiness and health checks; shutdown and upgrade mechanics; enforcement of exclusive active ownership; recovery reconciliation; and PostgreSQL availability.

Validation scenarios: Process failure and automatic restart; recovery of accepted messages; reconnection of agents already running; uncertain external-action outcomes; unavailable PostgreSQL during startup; and replacement without competing active orchestrators.
