# ADR 0059: Orchestrator-managed file storage on a persistent volume

Status: Accepted

Date: 2026-10-23

Shared skill bundles, attachments, and published agent outputs require durable file storage available independently of individual agent pods. Cubebot should keep its deployment dependencies small.

Decision: Store shared file bytes on a persistent volume managed by the orchestrator and expose access through an authenticated API that agents and human clients can use. The user selected this model on 2026-10-23, describing it as an internal object-storage facility intended to reduce dependencies. An external S3-compatible service is not required. [ADR 0060](0060-native-storage-api.md) selects a native Cubebot API without S3 wire compatibility initially.

Ownership: A storage module inside the existing Rust orchestrator owns transfers, file publication, integrity information, and physical cleanup. PostgreSQL remains authoritative for metadata, lifecycle records, and relationships to domain resources. Domain authorization determines who may read, publish, or remove a file. The persistent volume stores bytes; it is not mounted into agent pods for direct shared access.

Usage: An agent produces a report in its own persistent workspace, uploads it through the orchestrator, and attaches the returned artifact reference to an authorized task or room. A user can later download that published report while the agent sleeps. Skill imports and revisions use the same storage module for bundles, with their lifecycle controlled by the skills module.

Agent workspaces remain separate under ADR 0008. This facility stores shared published content and does not replace local workspace filesystem operations or automatically synchronize every agent file. Downloading an object does not confer access to unrelated files or bypass the resource's permissions.

Durability requirements: Incomplete uploads must not appear as completed artifacts. Acknowledged publication requires the file and metadata to satisfy a documented durability contract. Recovery must reconcile interrupted publication, lost responses, stale uploads, and unreferenced content. A database transaction cannot atomically commit filesystem changes. Physical deletion must respect live references and any retained revisions or backup requirements.

Capacity requirements: Bound transfer memory, concurrent streams, staging space, final storage use, and background maintenance. Large transfers must not monopolize the orchestrator's control work. Storage exhaustion, integrity failure, and unavailable volumes must produce explicit outcomes. Representative load and recovery tests are required before claiming support for a given agent count or throughput.

Availability follows ADR 0011: one active orchestrator with restart recovery and accepted downtime. The deployment must preserve exclusive ownership of the mounted storage during replacement. Kubernetes volume access mode alone is not a substitute for the complete active-owner contract. Recovery depends on storage-driver and filesystem behavior, which require qualification.

Backups must cover PostgreSQL, this shared-file volume, and agent workspaces with a consistent restore procedure. [ADR 0064](0064-coordinated-backup-and-restore.md) assigns workflow coordination and restore validation to Cubebot. A persistent volume is not by itself a backup or a guarantee of replication. File cleanup must account for the database restore points the installation intends to retain.

Alternatives considered: Require S3-compatible storage, or support both object storage and filesystem backends initially. The selected volume-backed module reduces required services and keeps one storage implementation to qualify. Its API hides layout from consumers, leaving room for later changes based on measured needs.

Engineering proposal: [Shared storage design](../design/shared-storage.md) develops immutable object publication, bounded streaming, recovery, and backup mechanics. Those details are proposed, not additional user-selected product decisions.

Open details: Native transport and endpoint contracts; transfer and resume contract; volume requirements; publication and cleanup mechanics; access revocation during downloads; quotas; retention; backup tooling; and recovery and throughput targets.

Validation scenarios: Agent upload and authorized human download; sleeping-agent artifact access; rejected cross-scope access; interruption at each publication boundary; lost acknowledgement and retry; full disk and database outage; shared reference deletion; orchestrator replacement; and restoring matching metadata and bytes.
