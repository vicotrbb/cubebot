# ADR 0079: Retain durable data until explicitly deleted

Status: Accepted

Date: 2026-10-24

Decision: Retain durable Cubebot data until an authorized explicit deletion removes it. Do not enable automatic age-based expiry by default. Retiring an agent preserves its records and workspace. The user selected this default on 2026-10-24.

Scope includes captured conversation and tool-result history, task records and results, durable memories, published artifacts, shared skill records and bundles, and retained agent workspaces. Optional retention rules must be explicitly configured with a defined scope before they can remove durable data; this decision selects no expiry periods or enabled retention rules.

Usage: A person returns to a retired research agent months later and can inspect its retained conversations, work results, and memory, subject to current access permissions. Dreaming and context compaction may have improved derived knowledge, but age, inactivity, retirement, and completed tasks have not erased the underlying retained records.

Ownership: Domain modules authorize deletion and identify affected records and references. Storage owns physical reclamation after the deletion contract permits it. Memory and history enforce provenance and suppression rules. Agent lifecycle releases execution resources without treating retirement as a data-deletion instruction. Backup management owns backup-copy retention and recovery behavior.

This is a retention default, not an immutable filesystem or a requirement to keep every historical file version. Ordinary authorized workspace edits, replacements, and explicit file removals remain possible. Memory correction and consolidation remain governed by ADRs 0020–0027. Temporary staging, caches, and diagnostic logs need separate bounded cleanup policies; retained tool-result evidence and published outputs must not be reclassified as temporary merely to reclaim space.

Storage capacity remains finite. Apply quotas, show capacity pressure, and reject or pause affected writes or work when they cannot be stored safely. Do not silently delete retained history, memories, artifacts, or retired workspaces to admit new data. Bounded capture and redaction still apply before persistence; the interface must represent omitted or truncated output honestly.

Explicit deletion must have clear scope. Removing a memory from retrieval, deleting its source conversation, deleting an artifact, and deleting all of an agent's data are different operations. The specification must define treatment of derived memories, embeddings, active contexts, references, and later extraction for each operation. A missing source must be shown as unavailable rather than cited as retrievable evidence. Existing correction and suppression protections must survive stale writes and background processing.

Deletion in the active installation does not by itself establish erasure from backups or external systems. Document backup retention, restoration behavior, and any deletion records needed to prevent unintended reappearance during recovery. Do not promise immediate physical erasure of every copy or deletion of externally published material through a local deletion operation.

Alternatives considered: Enable configurable age-based expiry by default, with separate source and derived-memory lifetimes. The user selected preservation until explicit deletion to support persistent agents and inspectable evidence.

Related decisions: ADR 0020 preserves source history during consolidation; ADR 0026 covers human memory correction; ADR 0018 distinguishes retirement from deletion; ADRs 0059–0060 define shared storage; ADRs 0064–0065 define coordinated backup and restore.

Open details: Deletion permissions and scope; derived-data propagation and suppression; workspace access for retired agents; backup-copy handling; optional policy interfaces; quotas and capacity-pressure behavior; physical reclamation; and temporary-data cleanup periods. These mechanics will be proposed in the consolidated specification.

Validation scenarios: Retain old conversations and retired workspaces; consolidate without source deletion; enforce current access to retained records; report capacity pressure without silent eviction; distinguish cache cleanup from durable-data removal; perform scoped authorized deletion; reconcile concurrent extraction and references; and restore without misrepresenting deleted data or backup-copy erasure. Implementation remains pending.
