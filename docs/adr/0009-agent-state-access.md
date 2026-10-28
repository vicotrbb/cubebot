# ADR 0009: Agent state access through the orchestrator

Status: Partially superseded by ADR 0083

Date: 2026-10-23

Superseded in part by [ADR 0083](0083-agent-centric-execution-in-a-sandbox-boundary.md): the agent, not an orchestrator module, owns memory behavior, and work reporting no longer depends on execution leases. Agents still reach platform state only through authenticated orchestrator operations and never through PostgreSQL.

[ADR 0008](0008-durable-storage.md) establishes PostgreSQL as the authoritative structured store. Agent pods execute model and tool work, while the orchestrator coordinates mailboxes, rooms, permissions, and lifecycle. State access needs a clear owner for authorization and persistence rules.

Decision: Agent pods access durable platform state through authenticated orchestrator operations. The user selected this model on 2026-10-23. Agent pods receive no platform database credentials and do not depend on its schema.

Usage: An agent requests relevant memories, sends a message, or records task progress through the corresponding orchestrator interface. For example, sending a message expresses the recipient and content; the messaging module owns permission checks, durable recording, duplicate handling, and delivery scheduling. Exact wire contracts, identifiers, and retry semantics will be specified later.

Ownership: Orchestrator modules own authorization, valid durable state transitions, and database transactions for their respective domains. The agent engine owns local reasoning and tool-loop progression and reports results through those interfaces. The memory module owns validated commits to shared agent memory. Database details stay inside the implementations of these modules.

An agent's authenticated identity constrains its operations. Sharing an installation does not authorize access to every agent's records. Work assigned to an execution must also remain valid when it reports progress; the lease and revocation mechanisms will be designed with recovery.

Rationale: A small domain-oriented interface hides persistence and coordination rules from agents. It keeps the implementation of those rules local and avoids distributing database credentials, schema knowledge, and connection management across agent pods.

Alternative considered: Direct PostgreSQL access from agents with scoped credentials would avoid the intermediary but distribute persistence knowledge and enforcement responsibilities. The user selected orchestrator-mediated access instead.

Consequences: Durable state operations require orchestrator availability and add a network exchange. A lost response may leave the caller uncertain whether an operation committed. Contracts must support safe retries and reconciliation rather than assume a network timeout means failure. Access mediation does not imply one orchestrator replica or one database connection.

Scope: This decision concerns Cubebot's structured platform state. It does not route every model request, terminal command, browser operation, or workspace file access through the orchestrator. [ADR 0031](0031-direct-inference-and-propagated-authentication.md) explicitly selects direct provider inference from agents with orchestrator-propagated authentication. Local caches, if justified later, cannot silently become competing authoritative stores.

Related decision: [ADR 0012](0012-agent-disconnection-behavior.md) permits bounded continuation of current turns during disconnection, using a durable local progress journal and subsequent orchestrator reconciliation. Agent pods still have no direct platform database access.

Open details: Transport, authentication and credential lifecycle, execution leases, idempotency contracts, streaming and batching, backpressure, and connection recovery.

Validation scenarios: Authorized memory retrieval; rejected cross-agent access; duplicate message submission after a lost acknowledgement; stale execution reporting progress; orchestrator outage during a task; and recovery without duplicate durable transitions or falsely acknowledging unsaved work.
