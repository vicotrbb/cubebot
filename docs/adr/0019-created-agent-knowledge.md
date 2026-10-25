# ADR 0019: Fresh memory with an explicit briefing

Status: Accepted

Date: 2026-10-23

Authorized agents can create persistent specialists and task-bound workers. Each new agent needs relevant starting context while preserving the independent memory ownership established in [ADR 0004](0004-unified-agent-memory.md).

Decision: Newly created agents start with fresh memory and an explicit briefing. The user selected this model on 2026-10-23. Creation does not copy a snapshot of the creator's saved memories or conversation history. The creator supplies the role, task, selected facts, and authorized references needed for the work.

The briefing is recorded as supplied context with its source identified. It is not inherited conversation history and does not assert that the new agent personally performed the creator's earlier work. The new agent develops its own memories through the ordinary memory mechanisms. Briefing format, size limits, and extraction into durable memories remain to be specified.

Usage: A lead creates a researcher with the investigation objective, findings so far, unresolved questions, and relevant document references. The researcher starts with that context and may ask follow-up questions through explicit messaging. It has no implicit access to the lead's other memories or projects.

A reference does not grant access to its target. The orchestrator validates the permitted sharing and the recipient's resource access. Model selection, role configuration, tool permissions, credentials, room membership, and workspace files remain separate from learned-memory initialization. The selected fresh-memory model applies to both lifetimes from [ADR 0018](0018-created-agent-lifetime.md).

Ownership: Creation records the new identity and briefing. The memory module owns the new agent's independent memory store. The messaging and task modules carry subsequent clarification and results. Knowledge explicitly communicated to the new agent is available across its contexts under ADR 0004.

Rationale: Focused initialization gives the worker the information needed for its purpose without copying unrelated history or introducing snapshot and synchronization rules.

Alternatives considered: A snapshot of the creator's memories would provide broad background but also copy irrelevant or stale information. Supporting both briefing and snapshot modes would add copying and provenance behavior. The user selected briefing-only initialization for the current architecture.

Consequences: A useful briefing becomes part of successful delegation. Missing context can require follow-up messages or retrieval through authorized tools. Parent and child memories evolve independently; later corrections or discoveries do not automatically synchronize between them. No automatic memory merge on child retirement is implied.

Open details: Briefing representation and limits; reference validation; durable fact extraction; context selection; and follow-up behavior while the creator is unavailable.

Validation scenarios: A new agent receives the supplied briefing but no unrelated creator memories; authorized references can be resolved and unauthorized references cannot; repeated creation delivery does not duplicate initialization; the new agent attributes received findings correctly; and subsequent memory changes remain independent.
