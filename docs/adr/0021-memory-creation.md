# ADR 0021: Explicit and automatic memory creation

Status: Accepted

Date: 2026-10-23

Retaining source history under [ADR 0020](0020-memory-consolidation-and-source-history.md) preserves evidence but does not itself select useful long-term knowledge. Agents need intentional memory writes and a way to recover useful information they did not explicitly save.

Decision: Support agent-directed writes and automatic extraction through one memory module. The user selected this model on 2026-10-23. Working agents can explicitly save important knowledge. Background processing can extract useful facts, decisions, preferences, procedures, and unresolved commitments from recorded work. Dreaming subsequently consolidates this derived knowledge.

Both paths use the same interface for validated memory changes and the same ownership, provenance, duplicate-handling, and conflict rules. They do not maintain separate authoritative memory pools. The memory module persists accepted changes through the orchestrator into PostgreSQL, consistent with ADRs 0008 and 0009.

Explicit writes follow the per-turn staging and successful-completion commit model in [ADR 0005](0005-agent-execution-lanes.md). Disconnected changes remain staged under ADR 0012. Automatic extraction is separately scheduled and committed against recorded source state; it must not bypass turn outcome checks by treating an unsuccessful operation as completed work.

Automatic extraction is selective and budgeted. It need not run after every message, and a successful extraction may produce no new memories. Source references and extraction identity must allow retries without duplicate writes. Background candidates and explicit writes must reconcile concurrent changes rather than silently overwrite newer state.

Usage: During a project discussion, an agent deliberately saves a chosen database requirement. Later extraction identifies an unresolved migration commitment from the recorded conversation. Both enter the same agent-owned memory store with source references. Dreaming can consolidate duplicates and reconcile later corrections without deleting the original exchanges.

Ownership: The working engine chooses explicit writes. Scheduled extraction produces candidate changes. The memory module owns validation and commit, and scheduling owns background admission and budgets. [ADR 0029](0029-model-default-and-purpose-overrides.md) selects the agent's default model unless an extraction override is configured. Execution placement and the precise scheduling mechanism remain open; this decision does not require another deployed process or database.

Rationale: Explicit writes capture the working agent's intent, while automatic processing can recover useful information it omitted. A shared commit interface keeps consistency rules in one place.

Alternatives considered: Explicit writes alone reduce background inference but depend on the working agent noticing every useful fact. Automatic extraction alone lacks an intentional save mechanism during work. The user selected both paths with shared rules.

Consequences: Extraction adds inference and operational cost. Stored interpretations may be incomplete or wrong, so provenance and correction behavior remain essential. Extraction must preserve uncertainty and distinguish reported claims, decisions, and observed outcomes rather than automatically converting every statement into fact. Memory quality needs evaluation independently of successful persistence.

Related decisions: [ADR 0022](0022-hybrid-memory-retrieval.md) combines keyword search, semantic similarity, and structured filters through the memory module. [ADR 0025](0025-idle-dreaming-and-visible-results.md) makes broader dreaming maintenance idle-only, with visible status and inspectable results; automatic extraction remains separately scheduled.

Human corrections: [ADR 0026](0026-human-memory-editing.md) adds direct editing and conversational correction through the same memory module. Extraction must respect committed revisions and recorded correction or removal intent when reprocessing rejected evidence.

Revision policy: [ADR 0027](0027-evidence-based-memory-revision.md) allows new evidence to justify revising a human-corrected memory while preserving attribution and revision history.

Open details: Memory types and schema; extraction triggers and batching; concrete model qualification and budgets; duplicate and contradiction handling; evidence assessment; deletion suppression mechanics; and quality acceptance criteria.

Validation scenarios: Explicit save and commit; useful automatic extraction without an explicit save; no-op extraction; duplicate candidate from both paths; concurrent correction; retry of the same source batch; failed-turn mutation exclusion; and preservation of source evidence through consolidation.
