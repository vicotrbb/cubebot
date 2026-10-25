# ADR 0020: Preserve source history during memory consolidation

Status: Accepted

Date: 2026-10-23

Cubebot manages bounded working context and maintains long-term agent memory. Dreaming consolidates knowledge, reconciles contradictions, summarizes history, and improves retrieval. These operations must distinguish derived knowledge from its source evidence.

Decision: Preserve source history while optimizing derived memory and context. The user selected this model on 2026-10-23. Dreaming and active context maintenance may replace summaries, consolidate derived memories, and exclude irrelevant material from working context. They do not delete original conversation records or recorded tool results merely because those records were summarized or judged unimportant.

Source history remains retrievable until an explicit retention or deletion policy removes it. Preserving history does not require including it all in every model call or keeping every byte in hot storage. [ADR 0079](0079-explicit-deletion-for-durable-data.md) selects retention until explicit deletion, with no automatic age-based expiry enabled by default. Storage limits and archival behavior remain open.

Derived memories and summaries retain references to supporting sources. Consolidation must preserve enough provenance to inspect how a conclusion was formed and to revisit details omitted from its summary. A model-generated summary is a derived interpretation and must not silently replace the original record as evidence.

Usage: A long investigation is condensed into a project summary and selected durable facts. Subsequent turns use that smaller context. If a later question requires an exact constraint or earlier reasoning, the agent can retrieve the relevant recorded exchange. Replacing the summary does not remove that exchange.

Ownership: Memory owns validated updates to derived knowledge and summaries. History owns source records and retrieval. Context assembly selects what fits the current turn. Authorized retention and deletion operations own removal of source data; dreaming has no independent authority to prune it. These are logical responsibilities, not a requirement for separate deployed processes or databases.

Rationale: Preserving sources allows recovery of omitted details, inspection of mistaken summaries, and changes to retrieval or consolidation methods without relying solely on previous model interpretations.

Alternative considered: Letting dreaming delete original history after extraction would reduce stored data but make mistaken judgments irreversible. The user selected source preservation with separate retention controls.

Consequences: Source storage grows independently of working context. Backups, retention, deletion, and archive retrieval need explicit design. Retention can remove a memory's supporting source, so references must handle that state without claiming missing evidence remains available. Deletion and suppression rules must also define treatment of derived memories and later re-extraction.

This decision concerns preserving captured records. It does not require storing hidden model reasoning, secrets, or unlimited tool output. Capture limits and redaction are separate ingestion requirements and must be represented honestly when material was omitted.

Related decision: [ADR 0024](0024-rolling-context-compaction.md) selects rolling summaries with recent exchanges retained for active context management.

Open details: Source and derived-record schemas; summary versioning; retention-policy mechanics; archive placement; source lookup; deletion propagation; context budgets; consolidation scheduling; and recall and summary-fidelity acceptance criteria.

Validation scenarios: Consolidate a conversation without deleting its source records; retrieve an omitted detail from original history; replace a summary while preserving provenance; apply explicit retention; resolve a reference to a removed source honestly; and keep working context bounded as retained history grows.
