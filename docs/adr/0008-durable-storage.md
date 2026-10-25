# ADR 0008: Durable storage

Status: Accepted

Date: 2026-10-23

Agents can sleep and restart while their histories remain available and new work continues to arrive. Rooms, mailboxes, concurrent task lanes, and the management console need a durable view of structured state independent of agent pod availability.

Decision: Use shared PostgreSQL as the authoritative store for structured Cubebot state, with a persistent workspace for each agent. The user selected this model on 2026-10-23.

PostgreSQL stores agent configuration and lifecycle records, permissions, memories, conversations, room records, mailbox deliveries, tasks, and execution checkpoints. Agent working files live in persistent workspaces and survive ordinary pod replacement and idle shutdown. Exact schema, retention, large-payload placement, and workspace provisioning remain open.

An agent's memory remains logically owned by that agent, as established in [ADR 0004](0004-unified-agent-memory.md). Sharing a database does not create shared memory between agents or grant additional access. The database is shared within one installation under [ADR 0001](0001-installation-and-tenancy.md).

Usage: While an agent sleeps, the console reads its retained structured history and the orchestrator records new deliveries. On wake, execution loads the required state and reattaches the agent's persistent workspace. Browsing workspace files while the agent sleeps requires a separate access design; choosing PostgreSQL does not solve that filesystem operation.

Rationale: One authoritative structured store supports coordination and access while agent pods are absent, without synchronizing authoritative histories across per-agent databases. PostgreSQL accommodates a shared control plane with concurrent writers. This choice does not itself establish control-plane replication, database high availability, or measured capacity.

Alternatives considered: Central SQLite behind one active control-plane instance would simplify deployment but constrain concurrent writers and complicate a later high-availability design. Central coordination storage plus authoritative SQLite per agent would distribute ownership physically but add cross-store coordination, backup, and sleeping-agent access work. Neither is part of the selected initial design.

Consequences: Installation requires PostgreSQL and persistent workspace storage. Database backups alone cannot restore workspace files. Recovery must account for both, and a database transaction does not atomically commit filesystem changes or external tool effects. Disposable caches may be added for demonstrated needs, but no additional database engine or cache is required by this decision.

Related decision: [ADR 0009](0009-agent-state-access.md) requires agent pods to access structured platform state through authenticated orchestrator operations, without direct database access.

[ADR 0059](0059-orchestrator-managed-file-storage.md) places shared skill bundles, attachments, and published artifacts on an orchestrator-managed persistent volume behind an authenticated API. PostgreSQL stores their metadata and domain references; agent workspaces remain separate.

[ADR 0063](0063-bundled-and-external-postgresql.md) makes bundled PostgreSQL the default deployment mode and supports an externally provisioned compatible database through the same persistence contract.

[ADR 0074](0074-postgresql-owned-agent-definitions.md) confirms PostgreSQL ownership of agent definitions and lifecycle intent, with Kubernetes resources reconciled from those records and no required Agent custom resource.

Open details: Persistence module interfaces and schema; connection pooling; database deployment mechanics and availability; workspace storage classes and provisioning; backup and restore consistency; and shared-file storage mechanics.

Validation scenarios: Read conversations while an agent sleeps; queue work while its pod is absent; restore an agent's state and files after pod replacement; commit concurrent lane updates without lost state; reject unauthorized cross-agent reads; and recover database plus workspace data from a documented backup procedure.
