# Memory, skills and files

This document records how each agent keeps knowledge: one memory pool per agent, how memories are created, retrieved, compacted, consolidated by dreaming and corrected by people, and what a newly created agent starts with. It also covers reusable skills and the shared skill library, durable structured storage, shared file storage and its API, and the default of keeping durable data until someone explicitly deletes it.

Since [Agent-centric execution in a sandbox boundary](sandbox.md#agent-centric-execution-in-a-sandbox-boundary), the agent owns its memory behavior (what to save, extract, retrieve and consolidate, and when). The orchestrator is the ledger: it durably records memory revisions, human edits, dream reports and skill records, enforces ownership, content permissions, revision checks and idempotency, and does not decide memory policy. Agents still reach that state only through authenticated orchestrator operations, never through PostgreSQL. Where the sections below name a "memory module", read it as the shared commit contract both sides use: the agent proposes changes, the ledger validates and commits them.

## Unified memory per agent

Accepted 2026-10-22. Amended 2026-10-26: each agent has exactly one permanent conversation, so references to several conversations for one agent no longer apply.

**Decision.** Each agent has one unified knowledge pool used across all its scopes and scenarios: its [one permanent conversation](agents.md#one-permanent-conversation-per-agent), shared rooms, direct agent messaging and tasks. The user selected this model on 2026-10-22 and explicitly confirmed that each agent's own memory is used everywhere it operates. Memory is not partitioned by user, room, conversation or task audience.

The agent may retrieve and use relevant memories acquired in any of its contexts. Summaries, extracted knowledge and memories produced during dreaming follow the same model. Unified availability does not require loading all memory into every model call; context selection and compaction remain runtime responsibilities.

Access controls on conversation transcripts do not guarantee that information from those conversations stays confined to their original audience. An agent may use what it learned in a private exchange when answering in a shared room. The console must explain this distinction. Work requiring separate knowledge pools should use separate agents; transfers between agents still require their own communication and access rules.

Scope: this does not create an installation-wide memory pool or automatically grant one agent access to another agent's memories. It does not grant additional tool, credential or resource permissions. Human access to stored transcripts, memories and files remains governed by the permission model, including [administrator content access](identity.md#administrator-content-access).

[Preserve source history during memory consolidation](#preserve-source-history-during-memory-consolidation) keeps source history while dreaming optimizes derived memory and context; source removal follows separate retention or deletion policy. [Explicit and automatic memory creation](#explicit-and-automatic-memory-creation) combines explicit saves and automatic extraction under one set of commit rules.

**Rationale.** The selected model gives each agent continuous knowledge and experience across its work, with one memory ownership rule. Audience-scoped knowledge was considered: it would restrict retrieval according to the current conversation's permissions and require explicit sharing across contexts. It was declined in favor of unified knowledge per agent.

**What changed.** The original text spoke of an agent's several private conversations. [One permanent conversation per agent](agents.md#one-permanent-conversation-per-agent) replaced that: every agent has exactly one permanent conversation, and context is managed within it. The memory rule itself is unchanged.

**Open details.** Memory representation, retrieval, extraction mechanics, consolidation mechanics, retention, deletion, concurrent updates and the mechanics of inter-agent knowledge transfer are specified separately (the sections below settle the selected models for creation, retrieval, compaction, consolidation, dreaming, correction and retention; their own mechanics remain open there).

**Validation scenarios.** An agent can recall a fact learned in its direct conversation in a room and in a later task; consolidation preserves knowledge without introducing audience partitions; another agent cannot automatically retrieve that memory; direct transcript access still follows permissions. Retrieval correctness and bounded context use receive separate acceptance criteria.

## Explicit and automatic memory creation

Accepted 2026-10-23. Amended 2026-10-27: the agent, not an orchestrator module, owns memory behavior, including when extraction runs.

**Decision.** Support agent-directed writes and automatic extraction through one memory commit contract. The user selected this model on 2026-10-23. Working agents can explicitly save important knowledge. Background processing can extract useful facts, decisions, preferences, procedures and unresolved commitments from recorded work. Dreaming subsequently consolidates this derived knowledge.

Both paths use the same interface for validated memory changes and the same ownership, provenance, duplicate-handling and conflict rules. They do not maintain separate authoritative memory pools. Accepted changes are persisted through the orchestrator into PostgreSQL, consistent with [Durable storage](#durable-storage) and [Agent state access through the orchestrator](agents.md#agent-state-access-through-the-orchestrator).

Explicit writes follow the per-turn staging and successful-completion commit model of [Agent execution lanes](agents.md#agent-execution-lanes). Changes made while disconnected stay staged in the agent's local journal until the orchestrator validates and commits them ([Bounded continuation during disconnection](agents.md#bounded-continuation-during-disconnection)). Automatic extraction is separately scheduled and committed against recorded source state; it must not bypass turn outcome checks by treating an unsuccessful operation as completed work.

Automatic extraction is selective and budgeted. It need not run after every message, and a successful extraction may produce no new memories. Source references and extraction identity must allow retries without duplicate writes. Background candidates and explicit writes must reconcile concurrent changes rather than silently overwrite newer state.

Usage: during a project discussion, an agent deliberately saves a chosen database requirement. Later extraction identifies an unresolved migration commitment from the recorded conversation. Both enter the same agent-owned memory store with source references. Dreaming can consolidate duplicates and reconcile later corrections without deleting the original exchanges.

Ownership: the working agent chooses explicit writes and, inside its own loop, when extraction runs and within what budget. Extraction produces candidate changes. The ledger validates and commits them, enforcing ownership, provenance, revision checks and recorded suppression; it does not schedule extraction or decide memory policy. [Default agent model with optional purpose overrides](agents.md#default-agent-model-with-optional-purpose-overrides) selects the agent's default model unless an extraction override is configured. This requires no further deployed process or database.

Extraction adds inference and operational cost. Stored interpretations may be incomplete or wrong, so provenance and correction behavior remain essential. Extraction must preserve uncertainty and distinguish reported claims, decisions and observed outcomes rather than turning every statement into fact. Memory quality needs evaluation independently of successful persistence.

Human corrections: [Direct memory editing and conversational correction](#direct-memory-editing-and-conversational-correction) adds direct editing and conversational correction through the same contract. Extraction must respect committed revisions and recorded correction or removal intent when reprocessing rejected evidence. [Revise human-corrected memories from new evidence](#revise-human-corrected-memories-from-new-evidence) lets new evidence justify revising a human-corrected memory while preserving attribution and revision history. [Hybrid memory retrieval](#hybrid-memory-retrieval) covers search; [Idle-only dreaming with visible status and results](#idle-only-dreaming-with-visible-status-and-results) makes broader dreaming maintenance idle-only, while automatic extraction remains separately scheduled.

**Rationale.** Explicit writes capture the working agent's intent, while automatic processing can recover useful information it omitted. A shared commit interface keeps consistency rules in one place. Explicit writes alone reduce background inference but depend on the working agent noticing every useful fact. Automatic extraction alone lacks an intentional save mechanism during work. The user selected both paths with shared rules.

**What changed.** The original assigned validation and commit to an orchestrator memory module and background admission and budgets to orchestrator scheduling, and left execution placement and the scheduling mechanism open. [Agent-centric execution in a sandbox boundary](sandbox.md#agent-centric-execution-in-a-sandbox-boundary) moved memory behavior and lane scheduling into the agent's loop, so the agent decides when extraction runs and the orchestrator keeps only the ledger role. That settles execution placement (inside the agent); the precise scheduling mechanism stays open. The original's "disconnected changes remain staged" still holds; the finite authorization window it relied on was removed by the same decision.

**Open details.** Memory types and schema; extraction triggers and batching; the precise mechanism by which the agent loop schedules extraction; concrete model qualification and budgets; duplicate and contradiction handling; evidence assessment; deletion suppression mechanics; and quality acceptance criteria.

**Validation scenarios.** Explicit save and commit; useful automatic extraction without an explicit save; no-op extraction; duplicate candidate from both paths; concurrent correction; retry of the same source batch; failed-turn mutation exclusion; and preservation of source evidence through consolidation.

## Hybrid memory retrieval

Accepted 2026-10-23.

**Decision.** Use hybrid retrieval, combining keyword search, semantic similarity and structured filters. The user selected this model on 2026-10-23. The orchestrator's memory interface combines eligible candidates into a bounded result set with provenance, rather than requiring the caller to coordinate separate search systems.

Usage: a query about data residency can retrieve a relevant memory phrased as keeping customer data in Brazil. A query for a specific project identifier can use lexical matching or identifier filters. Results include enough identity and source information to inspect or retrieve the supporting evidence. These are target scenarios for evaluation, not guarantees about a chosen embedding model.

The caller supplies its authenticated agent context, query, optional filters and result budget. The orchestrator derives or validates memory ownership from the authenticated request. Candidate retrieval and ranking must respect the permitted ownership scope. Hybrid search does not create a shared installation-wide memory pool or let caller-supplied identifiers bypass access checks.

Keyword matching supports lexical recall, semantic representations support conceptual similarity, and structured metadata narrows eligible records. Exact matching, ranking fusion, deduplication, recency handling and treatment of superseded memories are specified and evaluated separately. Similarity is a retrieval signal, not evidence that a memory is correct.

Implementation direction: use PostgreSQL's built-in full-text search and pgvector for vector similarity in the existing structured store. This avoids a separate search database. Extension version, embedding provider or local model, vector representation and index configuration remain open implementation choices. Hybrid retrieval is the accepted contract; detailed dependency qualification is still required, including extension availability in both [bundled and external PostgreSQL](packaging.md#bundled-postgresql-by-default-with-external-postgresql-support). No additional LLM reranker is required initially.

Ownership: the memory interface owns retrieval, candidate combination, filtering and the result contract. The agent's context assembly owns selection within the model's overall context budget. Search indexes and embeddings are derived data; source records and memory provenance remain authoritative under [Preserve source history during memory consolidation](#preserve-source-history-during-memory-consolidation).

Embedding generation adds computation and model lifecycle management. Records can exist before their embeddings are ready. Degraded retrieval, indexing lag, model changes, deletion propagation and reindexing require explicit behavior. Retrieval quality must be measured using both precise-reference and conceptual-recall cases.

[Tool-initiated memory retrieval](#tool-initiated-memory-retrieval) makes explicit agent tool calls the only trigger for retrieval, with no automatic saved-memory injection before turns.

**Rationale.** Keyword search and structured filters alone are simpler but may miss paraphrased concepts. Semantic search alone can miss precise references and return loosely related material. The user selected their combination.

**Open details.** Embedding selection and placement; indexing cadence; memory versus source-history search coverage; ranking; fallback behavior; language support; result limits; and recall, latency and cost targets.

**Validation scenarios.** Exact project reference; paraphrased concept; no useful match; duplicate candidates from both search paths; superseded information; new memory awaiting embedding; deleted memory excluded from results; and cross-agent access denied before results are exposed.

Implementation references reviewed during design: [PostgreSQL full-text search](https://www.postgresql.org/docs/current/textsearch-intro.html) and [pgvector](https://github.com/pgvector/pgvector).

## Tool-initiated memory retrieval

Accepted 2026-10-23.

**Decision.** Search saved memory only when the agent explicitly requests it through a memory tool. The user selected this model on 2026-10-23. The runtime does not automatically search or inject saved memories before a turn. There is no automatically injected essential-memory tier.

[Hybrid memory retrieval](#hybrid-memory-retrieval) defines how memories are retrieved; this decision defines when saved knowledge enters an agent's working context. The agent decides when it needs prior knowledge and supplies a query and any supported filters. The orchestrator performs bounded hybrid retrieval, enforces ownership and returns results with provenance. Returned material enters the requesting execution lane as a tool result, subject to its context budget. Other lanes do not automatically receive those results.

The current conversation, assigned task, instructions and an explicit creation briefing remain ordinary execution inputs. Maintaining a bounded representation of that conversation is distinct from searching the long-term knowledge pool. Context maintenance must not introduce implicit saved-memory retrieval. [Rolling context compaction](#rolling-context-compaction) handles active context automatically.

Automatic extraction and dreaming under [Explicit and automatic memory creation](#explicit-and-automatic-memory-creation) may still process recorded material to maintain derived memory. They do not automatically inject the resulting knowledge into working turns. Memory maintenance and working-agent recall have separate triggers.

While the orchestrator is unreachable, an agent can keep using already loaded context ([Bounded continuation during disconnection](agents.md#bounded-continuation-during-disconnection)). New central memory queries are unavailable until connectivity returns; the runtime must report that condition rather than fabricate results or silently substitute an unapproved local search path.

Ownership: the agent exposes memory tools and decides when to call them. The orchestrator's memory interface owns search, filtering, provenance and result limits. The agent's context assembly owns fitting requested results into the active context.

An agent may overlook useful saved information because it never searches. Extracting more memories does not solve that by itself. Instructions and evaluation should cover when to search, how to refine an unsuccessful query, and how to tell an unavailable search from an empty result. These instructions guide agent behavior without adding a hidden automatic retrieval path.

**Rationale.** Automatic bounded recall before each turn, supplemented by explicit tools, was considered: it could surface relevant knowledge without the agent asking. Tool-only retrieval avoids routine search and unsolicited context additions, at the cost of making recall depend on the agent recognizing its need for prior knowledge. The user selected tool-only retrieval.

**Open details.** Tool names and schemas; result and query budgets; search guidance; unavailable-search behavior; and evaluation targets for appropriate search initiation and useful recall.

**Validation scenarios.** No search or saved-memory injection without a tool call; explicit search returns only eligible results; automatic extraction does not inject memories into a later turn; current-conversation compaction introduces no implicit recall; unrelated lanes receive no search results; empty results and connectivity failure remain distinguishable; and an agent can follow provenance to supported evidence through an authorized interface.

## Rolling context compaction

Accepted 2026-10-23. Amended 2026-10-27: disconnected continuation no longer has a finite per-turn authorization window.

**Decision.** Automatically compact older exchanges into a rolling summary while retaining recent messages and tool interactions, and continue the same execution lane with the smaller context. The user selected this approach on 2026-10-23, following a recommendation for a structured summary that preserves the objective, constraints, decisions, unresolved work and evidence references. Compaction never creates a new conversation or erases retained history ([One permanent conversation per agent](agents.md#one-permanent-conversation-per-agent)).

Compaction runs before the context limit at a safe point between model and tool interactions. Context budgeting must account for instructions, tool definitions, retained exchanges, summary and response capacity. Compaction must preserve valid tool-call/result relationships and must not cause completed actions to run again.

Usage: during a long implementation task, older investigations and resolved attempts are summarized. Recent exchanges remain available in detail. The agent continues with the same task identity, pending work and execution permissions. If an omitted detail becomes necessary, the agent can explicitly retrieve its source through an authorized tool.

Source history remains intact under [Preserve source history during memory consolidation](#preserve-source-history-during-memory-consolidation). The summary is derived context with references to the covered history. It is not a replacement for evidence, an automatic long-term memory write, or authority to change task status or grants.

Compaction uses the lane's existing conversation material and previous summary. It does not automatically search the saved knowledge pool or import another lane's context. Explicit recall remains governed by [Tool-initiated memory retrieval](#tool-initiated-memory-retrieval). Automatically maintaining current context and explicitly retrieving long-term memory are separate operations.

Ownership: the agent's context module owns budget assessment, compaction and context assembly for each lane. History and memory keep their storage responsibilities. Durable compaction state is written to the ledger through the orchestrator; while disconnected the agent relies on its local journal and reconciles idempotently afterwards. Compaction requires no further deployed process.

Implementation requirements: prepare and validate a replacement before switching the lane to it. A failed or interrupted compaction must leave the previous valid context recoverable. If no valid context fits the model budget, pause or report the failure rather than silently discard required instructions or repeatedly send oversized requests. The detailed recovery protocol remains to be designed.

[Default agent model with optional purpose overrides](agents.md#default-agent-model-with-optional-purpose-overrides) uses the agent's default model for compaction unless a purpose override is configured. Context budgeting must account for both the compaction model and the model continuing the work.

Compaction consumes inference time and budget. Repeated summaries can omit or distort details; source preservation enables recovery but does not guarantee that the agent notices an omission. Summary fidelity and continuation quality need evaluation across multiple compaction cycles.

**Rationale.** Rebuilding context around a structured task handoff after each compaction was considered. It makes continuation explicit but discards more conversational continuity. Rolling compaction retains recent exchanges alongside the summary and fits ongoing conversations as well as autonomous tasks.

**What changed.** The original tied disconnected continuation to a finite per-turn authorization window. [Agent-centric execution in a sandbox boundary](sandbox.md#agent-centric-execution-in-a-sandbox-boundary) removed that window; the local journal and idempotent reconciliation remain.

**Open details.** Token accounting across providers; trigger thresholds; retained-message budgets and recent-context allocation; summary schema and model qualification; oversized individual results; timeout and retry policy; summary versioning and recovery; and latency, cost and fidelity targets.

**Validation scenarios.** Continue a task through repeated compactions; retain a corrected requirement and unfinished work; preserve valid tool interactions; keep concurrent lanes separate; retain source references; avoid implicit memory search; recover after interrupted compaction without repeating an external action; and stop safely when a valid context cannot fit.

## Preserve source history during memory consolidation

Accepted 2026-10-23.

**Decision.** Preserve source history while optimizing derived memory and context. The user selected this model on 2026-10-23. Dreaming and active context maintenance may replace summaries, consolidate derived memories and exclude irrelevant material from working context. They do not delete original conversation records or recorded tool results merely because those records were summarized or judged unimportant.

Source history remains retrievable until an explicit retention or deletion operation removes it. Preserving history does not require including it all in every model call or keeping every byte in hot storage. [Retain durable data until explicitly deleted](#retain-durable-data-until-explicitly-deleted) selects retention until explicit deletion, with no automatic age-based expiry enabled by default.

Derived memories and summaries retain references to supporting sources. Consolidation must preserve enough provenance to inspect how a conclusion was formed and to revisit details omitted from its summary. A model-generated summary is a derived interpretation and must not silently replace the original record as evidence.

Usage: a long investigation is condensed into a project summary and selected durable facts. Later turns use that smaller context. If a later question needs an exact constraint or earlier reasoning, the agent can retrieve the recorded exchange. Replacing the summary does not remove that exchange.

Ownership: memory owns validated updates to derived knowledge and summaries. History owns source records and retrieval. Context assembly selects what fits the current turn. Authorized retention and deletion operations own removal of source data; dreaming has no independent authority to prune it. These are logical responsibilities, not separate deployed processes or databases.

Source storage grows independently of working context. Backups, retention, deletion and archive retrieval need explicit design. Retention or deletion can remove a memory's supporting source, so references must handle that state without claiming missing evidence remains available. Deletion and suppression rules must also define treatment of derived memories and later re-extraction.

This decision concerns preserving captured records. It does not require storing hidden model reasoning, secrets or unlimited tool output. Capture limits and redaction are separate ingestion requirements and must be represented honestly when material was omitted. [Rolling context compaction](#rolling-context-compaction) selects rolling summaries with recent exchanges retained for active context management.

**Rationale.** Preserving sources allows recovery of omitted details, inspection of mistaken summaries, and changes to retrieval or consolidation methods without relying solely on earlier model interpretations. Letting dreaming delete original history after extraction was considered: it would reduce stored data but make mistaken judgments irreversible. The user selected source preservation with separate retention controls.

**Open details.** Source and derived-record schemas; summary versioning; retention-policy mechanics; storage limits and archival behavior and placement; source lookup; deletion propagation; context budgets; consolidation scheduling; and recall and summary-fidelity acceptance criteria.

**Validation scenarios.** Consolidate a conversation without deleting its source records; retrieve an omitted detail from original history; replace a summary while preserving provenance; apply explicit retention or deletion; resolve a reference to a removed source honestly; and keep working context bounded as retained history grows.

## Idle-only dreaming with visible status and results

Accepted 2026-10-23. Amended 2026-10-27: the agent decides when it is idle enough to dream; the orchestrator only refuses a run while work is queued, running or waiting.

**Decision.** Dream only while the agent is idle across all execution lanes. Dreaming consolidates an agent's derived memory and can create or improve reusable skills ([Reusable skills created by humans, agents, and dreaming](#reusable-skills-created-by-humans-agents-and-dreaming)). The user selected idle-only dreaming on 2026-10-23 and explicitly required visibility into both the dreaming state and its results through the web console and mobile PWA.

Admission: lanes are a scheduling rule inside the agent loop, so the agent decides when it is idle enough to dream, and the orchestrator only refuses a run while work is queued, running or waiting. Dreaming must not start alongside an active main or task turn, or take precedence over eligible queued work. New work requests make dreaming yield at a safe checkpoint before ordinary execution begins. A quiet main lane alone does not establish idle eligibility. Unresolved execution outcomes must be reconciled before treating a lane as idle.

Under [Agent idle lifecycle](agents.md#agent-idle-lifecycle) agents are always on and an agent idle past its delay is sleeping; dreaming runs while it sleeps, subject to admission and budgets. Idle status does not trigger continuous inference. Retired agents remain ineligible ([Persistent and task-bound created agents](agents.md#persistent-and-task-bound-created-agents)). Automatic extraction ([Explicit and automatic memory creation](#explicit-and-automatic-memory-creation)) and necessary active-context compaction ([Rolling context compaction](#rolling-context-compaction)) remain separate operations; this decision governs broader dreaming maintenance.

Usage: a user sees an agent marked as dreaming, opens its current maintenance run and sees the recorded activity. When the run finishes, the user can inspect a concise result summary and the actual memory changes. If a message arrives first, the run yields and the agent handles the message. Its report distinguishes changes already committed from unfinished work.

Visibility contract:

- Agent lists and agent details distinguish dreaming from sleeping, ordinary work and unavailability. A run waiting for admission is not shown as actively dreaming.
- Each run has a durable identity, timestamps, status and recorded progress. Completion, yielding and failure remain distinguishable; stale or disconnected status must not masquerade as confirmed live progress.
- Results show a readable summary backed by committed changes: memories added, revised, merged or superseded, with before/after content where applicable and references to supporting sources. An unchanged run explicitly reports no changes. Unresolved contradictions can remain unresolved in the report.
- Failed or yielded runs expose any committed partial results separately from pending proposals. A model's narrative alone is not proof that a change was applied.
- Results also show skills created or revised, supporting sources, revision changes, installation outcomes and any validation actually performed. Saved revisions and active installations remain distinguishable.
- Past results remain inspectable while the agent sleeps. Result retention and pagination remain to be specified.

Reports expose outcomes and evidence, not hidden model reasoning. They follow existing content-access rules: administrators have full access under [Administrator content access](identity.md#administrator-content-access), while ordinary users need the relevant grants. Status visibility must not accidentally disclose protected memory content through previews or summaries.

Ownership: the agent owns idle eligibility and yielding; the orchestrator refuses a run while work is queued, running or waiting. The memory commit contract owns memory changes, provenance and conflict checks. The skills module owns skill revisions and installation outcomes. Run records associate committed changes with the originating dreaming run. The console reads these records through the orchestrator.

Dreaming preserves source history ([Preserve source history during memory consolidation](#preserve-source-history-during-memory-consolidation)) and does not inject memories into working contexts ([Tool-initiated memory retrieval](#tool-initiated-memory-retrieval)). On yielding or failure, incomplete changes must not appear committed. Retrying a run must not duplicate already committed changes. Idle-only execution does not remove the need to handle concurrent automatic extraction or other authorized memory updates.

Busy agents can accumulate a maintenance backlog; the console should expose delayed maintenance. Safe yielding can add some response latency, so checkpoint duration needs a bound. Reports and change records require storage and must stay consistent with actual commits.

[Direct memory editing and conversational correction](#direct-memory-editing-and-conversational-correction) supports editing what a report reveals, with an inspectable revision trail. [Default agent model with optional purpose overrides](agents.md#default-agent-model-with-optional-purpose-overrides) uses the agent's default model for dreaming unless a purpose override is configured.

**Rationale.** Concurrent dreaming could keep maintenance progressing for continuously busy agents, but adds competition with active work. Per-agent configurability would require supporting both modes. The user selected idle-only operation. A status badge alone would not satisfy the additional requirement to inspect results.

**What changed.** The original said "scheduling owns admission": an orchestrator scheduling module decided idle eligibility. [Agent-centric execution in a sandbox boundary](sandbox.md#agent-centric-execution-in-a-sandbox-boundary) made lanes a scheduling rule inside the agent loop, so the agent decides when it is idle enough to dream and the orchestrator only refuses a run while work is queued, running or waiting. The idle-only rule, yielding and the visibility contract are unchanged.

**Open details.** Trigger cadence; idle grace period; budgets and fairness; yield latency; maintenance backlog indicators; run and change-record schemas; ordinary-user access; report retention and pagination; live-update transport; event transport, checkpoint format and execution placement details; and rollback support.

**Validation scenarios.** An active task prevents dreaming; eligible work wins an admission race; a message causes safe yielding; committed partial results survive a crash; retries do not duplicate changes; no-op and failure outcomes are clear; reports match committed changes; authorized users inspect past results while the agent sleeps; unauthorized users cannot read protected results; and dreaming preserves source history.

## Direct memory editing and conversational correction

Accepted 2026-10-23.

**Decision.** Support both direct memory editing in the web console and mobile PWA and conversational correction through the agent. The user selected this model on 2026-10-23. Authorized users can revise or remove a memory directly, without requiring the agent to interpret the requested change. They can also ask the agent to perform the correction in chat.

Usage: a dreaming report reveals that an agent inferred the wrong project deadline. A user opens the affected memory, changes the deadline and saves the correction. Alternatively, the user tells the agent which deadline is wrong and asks for a correction. Both routes update the same agent-owned knowledge pool through the same commit contract.

Direct edits are authenticated domain operations through the orchestrator. They can complete while the agent sleeps. They record the human actor and become authoritative when the operation commits. Conversational changes retain the requesting user and source-message provenance and follow the agent's successful-turn commit rules ([Agent execution lanes](agents.md#agent-execution-lanes)). A proposed correction must not be presented as persisted before its commit succeeds.

Reading memory and editing memory are distinct permissions. Administrator read access ([Administrator content access](identity.md#administrator-content-access)) does not itself settle mutation rights. Conversational tools must not turn an unauthorized request into an authorized human correction. Ordinary agent learning remains governed by its own grants and memory policy.

Ownership: the memory commit contract owns mutation validation, revision checks, provenance and suppression rules for both routes. The console and conversational tools are callers of that interface. No separate editable copy of memory is introduced. A stale edit or background candidate must not silently overwrite a newer committed revision.

Corrections keep an inspectable revision trail with actor, time, source and changed content, subject to explicit retention and deletion rules. Removing a memory excludes it from active retrieval. It does not by itself erase original conversation history or promise erasure of all audit records, summaries and already-loaded contexts. The interface must communicate the scope of removal; broader erasure remains a separate design topic.

Later extraction or dreaming must respect recorded correction and removal intent when processing the same rejected evidence. Replaying old source material must not simply recreate the rejected memory as a new record. The representation of that intent and handling of paraphrased duplicates are implementation details requiring evaluation. [Revise human-corrected memories from new evidence](#revise-human-corrected-memories-from-new-evidence) permits revising a human-corrected memory when genuinely new evidence warrants it, with attribution and revision history preserved.

Editing durable memory does not retroactively change a running model request or an already-performed action. This decision adds no automatic saved-memory injection ([Tool-initiated memory retrieval](#tool-initiated-memory-retrieval)). Dreaming reports ([Idle-only dreaming with visible status and results](#idle-only-dreaming-with-visible-status-and-results)) are where users find affected memories; revision history does not imply a whole-run rollback feature.

The console needs revision-aware editing and clear conflict feedback. Automatic extraction and dreaming must reconcile their candidates against human corrections. Reliable suppression requires more than deleting a search-index entry.

**Rationale.** Conversational correction alone would reduce console functionality but make every correction depend on model interpretation and successful execution. Direct editing plus conversation gives users an exact edit path and a convenient natural-language path.

**Open details.** Editing permissions (the role-to-permission mapping); evidence assessment and contradiction handling; suppression representation; conversational target disambiguation; propagation to active contexts; correction notifications; revision retention; broader erasure; and rollback support.

**Validation scenarios.** Edit while the agent sleeps; correct through chat and verify commit; remove a memory from retrieval; reject an unauthorized direct or conversational mutation; detect a stale editor revision; reject an outdated extraction candidate; prevent replay of rejected source evidence from recreating the memory; and preserve accurate attribution for both correction paths.

## Revise human-corrected memories from new evidence

Accepted 2026-10-23.

**Decision.** Allow the agent and automatic memory maintenance to revise a human-corrected memory when new evidence warrants the change. The user selected this policy on 2026-10-23. A human edit does not make a memory immutable or require approval of each later revision.

Usage: a user corrects a project deadline to November 15. Later, a project update explicitly changes it to November 22. The agent can revise the memory, preserving the human correction in its revision history and citing the new update. Merely rereading the old, rejected deadline does not justify changing it back.

Newly retrieved does not mean new evidence. A later ingestion timestamp, another paraphrase of an old source, or a repeated model interpretation is insufficient by itself. Revision should account for what the evidence actually establishes, its applicability, and whether it supersedes the earlier fact. Recency alone does not establish truth. When evidence does not resolve a contradiction, the memory can retain the uncertainty rather than invent a definitive resolution.

Ownership: working agents and maintenance jobs propose revisions with evidence references. The commit contract enforces authorization, expected revisions, provenance and recorded suppression rules at commit. Assessing the meaning and reliability of evidence remains an inference task to evaluate; validating the record structure cannot guarantee semantic correctness.

Each revision retains the changed content, actor or process, time and supporting evidence under the revision-history policy. A concise explanation of the change should be inspectable without exposing hidden reasoning. Revisions made during dreaming appear in its committed-change report ([Idle-only dreaming with visible status and results](#idle-only-dreaming-with-visible-status-and-results)). No separate notification is required for every revision.

Stale proposals must not overwrite a newer human or agent edit. A proposal built against an obsolete revision must be reconciled before committing. Reprocessing the same rejected evidence remains subject to the correction and removal rules of [Direct memory editing and conversational correction](#direct-memory-editing-and-conversational-correction). This decision permits supported revisions to existing memories; it does not grant blanket permission to recreate explicitly removed memories or erase source history.

Memory is knowledge, not an authorization mechanism. Revising a remembered statement cannot change an enforced capability grant, credential scope, resource budget or configured instruction. Those keep their own owners and permission checks.

Human control therefore means inspection and further correction, not a guarantee that an edited value stays unchanged. An agent can still misjudge evidence and make an incorrect revision. Evaluation must cover misleading recent sources, repeated old sources, actual changes and unresolved disagreement.

**Rationale.** Two alternatives were considered and not selected: protect every human correction until a user changes it, or let users protect selected memories (an optional per-memory protection mode). Both give stronger control over individual values but can leave knowledge outdated or add another user-facing policy. The user selected revisable corrections with evidence and revision history.

**Open details.** Evidence and revision schemas; source reliability and temporal interpretation; contradiction representation; notification preferences; reconciliation behavior; and revision-quality acceptance criteria.

**Validation scenarios.** Revise a human-corrected deadline from a supported later update; preserve attribution and source references; refuse to treat re-ingested rejected evidence as new; retain an unresolved contradiction; detect a stale candidate after another edit; expose dreaming revisions in its report; and prevent memory revisions from altering enforced permissions.

## Fresh memory with an explicit briefing

Accepted 2026-10-23.

**Decision.** Newly created agents start with fresh memory and an explicit briefing. The user selected this model on 2026-10-23. Authorized agents can create persistent specialists and task-bound workers, and each new agent keeps the independent memory ownership of [Unified memory per agent](#unified-memory-per-agent). Creation does not copy a snapshot of the creator's saved memories or conversation history. The creator supplies the role, task, selected facts and authorized references needed for the work.

The briefing is recorded as supplied context with its source identified. It is not inherited conversation history and does not assert that the new agent personally performed the creator's earlier work. The new agent develops its own memories through the ordinary memory mechanisms.

Usage: a lead creates a researcher with the investigation objective, findings so far, unresolved questions and relevant document references. The researcher starts with that context and may ask follow-up questions through explicit messaging. It has no implicit access to the lead's other memories or projects.

A reference does not grant access to its target. The orchestrator validates the permitted sharing and the recipient's resource access. Model selection, role configuration, tool permissions, credentials, room membership and workspace files remain separate from learned-memory initialization. The fresh-memory model applies to both lifetimes in [Persistent and task-bound created agents](agents.md#persistent-and-task-bound-created-agents).

Ownership: creation records the new identity and briefing. The new agent owns its own independent memory. Messaging and task records carry later clarification and results. Knowledge explicitly communicated to the new agent is available across its contexts under [Unified memory per agent](#unified-memory-per-agent).

A useful briefing becomes part of successful delegation. Missing context can require follow-up messages or retrieval through authorized tools. Parent and child memories evolve independently; later corrections or discoveries do not synchronize between them. No automatic memory merge on child retirement is implied.

**Rationale.** Focused initialization gives the worker the information needed for its purpose without copying unrelated history or introducing snapshot and synchronization rules. A snapshot of the creator's memories would provide broad background but also copy irrelevant or stale information. Supporting both briefing and snapshot modes would add copying and provenance behavior. The user selected briefing-only initialization for the current architecture.

**Open details.** Briefing representation, format and size limits; reference validation; extraction of durable facts from the briefing; context selection; and follow-up behavior while the creator is unavailable.

**Validation scenarios.** A new agent receives the supplied briefing but no unrelated creator memories; authorized references can be resolved and unauthorized references cannot; repeated creation delivery does not duplicate initialization; the new agent attributes received findings correctly; and later memory changes remain independent.

## Reusable skills created by humans, agents, and dreaming

Accepted 2026-10-23. Amended 2026-10-27: skill scripts run in the agent's own sandbox rather than a separate tool environment.

**Decision.** Support reusable skills inspired by OpenBot's package and revision model. Skills are a core part of Cubebot: agents need reusable procedures, references and scripts alongside their standing instructions and unified memory, and successful work should improve how later work is done. Agents can create their own skills during work. Idle dreaming can create and refine skills to improve future work. Authorized humans can create, edit and import skills. The user explicitly selected these capabilities on 2026-10-23. Skill creation is an ordinary agent capability within its scope, without a per-creation approval queue.

Usage: after solving a recurring diagnostic problem, an agent saves a procedure with verification steps. During a later idle dream, it identifies a repeated successful workflow and creates or improves a skill, linking the change to supporting history. A human can inspect the result, edit it, or import an existing package for an agent.

Package model: a directory with a `SKILL.md` entrypoint containing YAML metadata and Markdown instructions, with optional scripts, references and assets. Follow the Agent Skills format used by OpenBot ([File and Git imports with Agent Skills compatibility](#file-and-git-imports-with-agent-skills-compatibility)). References resolve relative to the package. Cubebot's Rust agent loads skills itself, without provider CLI-specific directory copies.

Agents can discover eligible skills and load instructions and supporting files as needed. Full package contents do not belong permanently in every model request. Skills describe how to perform work; memories retain agent knowledge; tools perform operations. Each has its own lifecycle.

Revision model: immutable, identifiable revisions, and per-agent installations that select a revision. Creation registers the skill and installs it for the creating agent. Publishing a revision does not silently replace installed copies, including those of other agents. Updating an installation is a distinct operation permitted within the actor's scope. Existing executions must not unknowingly mix package revisions. If registration succeeds but installation fails, recovery uses the saved identity instead of creating another skill.

Ownership: a skills module in the orchestrator owns catalog metadata, package validation, revisions, installation state and provenance. The agent makes selected package files available in its sandbox. PostgreSQL remains authoritative for durable records ([Durable storage](#durable-storage)); bundle bytes live in shared file storage ([Orchestrator-managed file storage on a persistent volume](#orchestrator-managed-file-storage-on-a-persistent-volume)). Workspace edits are working copies until registered. Humans, agents and dreaming use the same revision contract with conflict detection.

Dreaming stays idle-only and yields ([Idle-only dreaming with visible status and results](#idle-only-dreaming-with-visible-status-and-results)). Reports include skills created or revised, revision identities, supporting source references, readable changes and installation outcomes. Committed results are separate from unfinished proposals. A claimed improvement is not proof of better performance: reports distinguish package validation, executed checks and untested expectations. Dreaming acquires no additional external-action authority to validate skills.

Importing or registering a package does not execute its scripts. Validate structure, paths, file types and size before acceptance. Scripts run in the agent's sandbox under existing execution, network and credential restrictions. Skill metadata cannot grant tools, override platform policy or register integrations. Integrity checks do not establish that instructions or scripts are safe or correct. Generated skills should generalize procedures without embedding credentials or unnecessary private source content.

Registered skills are shared and discoverable across the installation by default, with each agent selecting its own installations ([Shared installation skill library by default](#shared-installation-skill-library-by-default)). Any active agent or admitted active user may contribute revisions ([Shared skill revisions by any agent or user](#shared-skill-revisions-by-any-agent-or-user)). Removal and changes to another agent's installed skills keep separate authority. There is no public marketplace and no authority to publish outside the installation. Ecosystem packages are imported from their actual source locations.

**Rationale.** Instructions and memory alone, with ad hoc procedure files, were considered. The selected model adds an explicit reusable package, import, revision and installation lifecycle so successful work improves later work.

**What changed.** The original left bundle storage open; [Orchestrator-managed file storage on a persistent volume](#orchestrator-managed-file-storage-on-a-persistent-volume) settled it (bundle bytes on the shared volume, metadata in PostgreSQL). The original placed scripts in a separate tool environment; [Agent-centric execution in a sandbox boundary](sandbox.md#agent-centric-execution-in-a-sandbox-boundary) runs them in the agent's own sandbox under the same restrictions.

**Open details.** Management of another agent's installations; import mechanics and compatibility qualification; search and activation interface; bundle materialization in the sandbox; update activation points; disable and removal behavior; and quality evaluation.

**Validation scenarios.** Human creation and import; agent creation and installation; idle dreaming with visible skill results; stale-revision conflicts; unchanged installations after publication; partial-installation recovery; unsafe package rejection; bounded loading; and denied authority expansion through skill content.

Evidence: OpenBot revision `cfee2971f29cdb7c16272e60ae912de7fbb30693`, inspected locally on 2026-10-23: [skill architecture](https://github.com/nightly-labs/openbot/blob/cfee2971f29cdb7c16272e60ae912de7fbb30693/docs/architecture/skills-and-plugins.md), [creation and installation tools](https://github.com/nightly-labs/openbot/blob/cfee2971f29cdb7c16272e60ae912de7fbb30693/src/main/local-skill-tools.ts), [immutable revisions](https://github.com/nightly-labs/openbot/blob/cfee2971f29cdb7c16272e60ae912de7fbb30693/src/main/local-skill-library.ts) and [package validation](https://github.com/nightly-labs/openbot/blob/cfee2971f29cdb7c16272e60ae912de7fbb30693/src/main/skill-package.ts). The [Agent Skills specification](https://agentskills.io/specification) was reviewed for the package format. Dreaming-generated skills are a Cubebot requirement, not a verified OpenBot dreaming feature.

## Shared installation skill library by default

Accepted 2026-10-23.

**Decision.** Newly registered skills enter a shared installation library by default and are discoverable by other agents. Each agent independently selects which available skills and revisions to install. The user selected this model on 2026-10-23. Sharing within the installation does not publish a skill externally or install it everywhere automatically.

Usage: one agent develops a diagnostic skill and registers it during a dream. Another agent searches the library, reads the relevant package and installs its selected revision for future work. The first agent later publishes an improvement; the second agent's installation stays unchanged until it selects an update.

The default applies to skills created by agents during ordinary work or dreaming, and to skills registered by authorized humans through creation or import. Library discovery includes access to the package needed to evaluate and use it. Per-agent installation records remain separate from shared library records. Discovery, installation and execution grant no additional tools, credentials or resource access.

Ownership: the skills module owns shared catalog entries, immutable revisions and per-agent installation records. Any active agent or admitted active user may publish revisions ([Shared skill revisions by any agent or user](#shared-skill-revisions-by-any-agent-or-user)). Authorization keeps responsibility for separate operations such as removal or management of another agent's installations.

Reusable packages must be written with their shared audience in mind. An agent's private task history does not become generally readable merely because it produced a skill. Generated instructions should extract procedures without embedding credentials, private transcripts or unnecessary source-specific data. Internal provenance can link to protected source records, whose original access checks still apply. Discovering a package cannot authorize following a protected source link. Automated checks cannot guarantee that generated content contains no sensitive information, so sharing and provenance must stay inspectable.

Human members can read shared packages and contribute revisions. These capabilities do not grant unrestricted library administration or access to protected underlying histories. Administrators keep their existing content-access rights.

Skill registration and dream results must make shared-library publication visible, including identity and revision. Publication does not prove that other agents adopted the skill. A failed registration must not appear discoverable, and a successful registration followed by failed local installation must remain a distinct recoverable outcome.

Amended 2026-11-04: Cubebot ships default skills, `coding` and `design`, from `apps/orchestrator/skills`. The orchestrator adds each once at startup, installs it on every agent then and on each new agent after. They are ordinary library entries: people revise them, revisions pin as usual, and uninstalling one from an agent sticks. A name a person already took is left alone, and an upgrade never overwrites a shipped skill.

**Rationale.** Keeping skills private to the creating agent until explicitly shared was considered. The shared default supports installation-wide reuse and follows OpenBot's shared local-library direction while keeping independent installations.

**Open details.** Removal and other library-administration authority; any future restricted visibility option; catalog ranking; provenance presentation; and package withdrawal behavior.

**Validation scenarios.** Discover a newly created or imported skill from another agent; install a selected revision; keep other installations unchanged after publication; deny tool or source access not otherwise granted; distinguish shared publication from adoption; and keep contribution rights separate from removal and installation-management rights.

## Shared skill revisions by any agent or user

Accepted 2026-10-23.

**Decision.** Any active agent or admitted, active user in the installation can contribute a new revision to an existing shared skill. The user explicitly selected both agents and users on 2026-10-23. Contributions need no creator ownership, maintainer assignment or separate approval step before publication to the installation library.

Usage: agent B improves a procedure created by agent A and publishes a new revision. A human then corrects the procedure through the same revision interface. Both contributions keep attribution and prior versions. Agents using an earlier revision keep using it until their installation is updated.

Ownership: the skills module owns revision publication, immutable content, provenance and conflict handling. Platform identity supplies the authenticated actor. Authorization enforces active installation membership or agent identity and existing operational restrictions. Reading shared packages to contribute is available to installation members; this does not expose protected source history the package references.

Each revision records its contributor, parent revision, time and inspectable changes. Publication validates the package and the expected current revision. Concurrent contributors must not silently overwrite each other's work: a stale contribution receives a conflict and can be reconciled against the current version. Retrying publication after a lost response must not duplicate a committed contribution.

Agents may contribute during ordinary work and idle dreaming. Dream reports identify revisions actually saved and distinguish them from drafts or validation failures. Publication stays separate from installation, and neither contribution nor installation grants permission to run otherwise unauthorized tools or access referenced resources.

This authority allows new revisions only. It does not authorize rewriting old revisions, deleting the skill or its history, changing another agent's installed revision, or publishing externally; those keep their separate scopes. Imported skills can receive local revisions while keeping their recorded origin; a local contribution does not update the upstream source.

Package validation establishes structural acceptability, not correctness or improved performance. Contributor identity, history and any verification evidence stay visible so consumers can assess a revision before adopting it. The shared-content handling of [Shared installation skill library by default](#shared-installation-skill-library-by-default) still applies.

**Rationale.** Restricting revisions to creators and assigned maintainers was considered. The user selected open contribution across installation agents and users to support collective improvement.

**Open details.** Revision presentation; contribution metadata; conflict reconciliation interface; publication idempotency; removal and withdrawal authority; and update-adoption experience.

**Validation scenarios.** A non-creator agent publishes a revision; an ordinary active member publishes a correction; a suspended member is denied; stale publication reports a conflict; previous revisions remain readable; existing installations remain unchanged; and contribution rights do not permit deletion or access to protected provenance.

## File and Git imports with Agent Skills compatibility

Accepted 2026-10-23.

**Decision.** Support skill imports from files, folders, archives and Git repositories, compatible with the Agent Skills format documented at agentskills.io. The user selected these sources and explicitly included agentskills.io on 2026-10-23. Imported packages get the same revision, validation and installation behavior as locally authored skills.

Source clarification: the reviewed agentskills.io home page and documentation index describe the open format, authoring guidance and client integration. They document no hosted skill registry or package-download API. Cubebot therefore imports ecosystem skills from their actual package or repository locations. Importing the documentation home page itself is not an installation operation. A dedicated remote catalog adapter would need an actual catalog contract and a further decision.

Usage: a human uploads a standalone `SKILL.md`, selects a skill folder or archive, or supplies a Git repository and skill subdirectory. Cubebot inspects and validates the selected package, records its origin and exact content, then registers a revision in the shared library. Installation on an agent uses the existing revision-selection flow. A repository containing several skills must offer explicit package selection rather than silently importing every directory.

Ownership: source adapters obtain candidate files; the skills module owns normalization, validation, provenance and revision publication. Folder import means user-selected uploaded content or an authorized workspace source, not arbitrary access to the orchestrator's filesystem. The web console and PWA may offer different source-selection controls over the same backend contract.

Git imports record the resolved commit and selected path as well as the source reference. Uploaded packages keep content identity and available origin metadata. Imports create durable local snapshots: a moving branch, deleted upstream repository or later source edit must not silently change a registered revision. Later upstream imports must account for local contributions rather than overwrite them.

Use the Agent Skills structure: `SKILL.md` with metadata and instructions, plus optional supporting files. Preserve declared compatibility, attribution and license metadata where present. Format compatibility does not promise every imported procedure can run: provider-specific tools, missing software and unavailable integrations must be reported. Skill metadata, including tool declarations, cannot expand Cubebot grants. A standalone entrypoint with missing referenced files must surface that limitation rather than appear complete.

Fetch and validation use existing network and credential restrictions. Private repositories need applicable installation-managed access. Reject unsafe paths and unsupported file forms, bound transfers and expanded content, and execute no package scripts or repository hooks during import. Importing a skill does not register external integrations, install all its dependencies or publish local changes upstream.

**Rationale.** Two alternatives were considered: direct imports alone without an explicit ecosystem compatibility target, or adding external catalog browsing or a Cubebot-operated public marketplace. The selected scope is direct imports plus Agent Skills compatibility. No public marketplace is selected.

**Open details.** Supported archive and Git URL forms; repository access mechanics; multi-skill selection; import size limits; format compatibility policy; duplicate imports and deduplication; upstream update reconciliation; and import authority for agents.

**Validation scenarios.** Import a standalone entrypoint, complete folder, archive and selected Git subdirectory; preserve supporting files; record a resolved commit; detect missing references; reject path escape and oversized packages; preserve a local snapshot after upstream changes; and prevent import-time execution or authority expansion.

References reviewed on 2026-10-23: [Agent Skills overview](https://agentskills.io/home), [documentation index](https://agentskills.io/llms.txt) and [format specification](https://agentskills.io/specification).

## Durable storage

Accepted 2026-10-23. Amended 2026-10-28: the workspace is a per-agent volume attached to the sandbox (a guest disk under `kvm`) that also holds the recovery journal. Amended 2026-11-02: workspace layout is per runtime, and a sandbox refuses a volume the other runtime laid out.

**Decision.** Use shared PostgreSQL as the authoritative store for structured Cubebot state, with a persistent workspace for each agent. The user selected this model on 2026-10-23. Agents sleep and restart while their histories stay available and new work keeps arriving; rooms, mailboxes, concurrent lanes and the console need a durable view of structured state independent of whether the agent's sandbox is running.

PostgreSQL stores agent definitions and lifecycle records, permissions, memories, conversations, room records, mailbox deliveries, tasks and execution checkpoints. PostgreSQL also owns agent definitions and lifecycle intent, with Kubernetes resources reconciled from those records and no required Agent custom resource ([PostgreSQL owns agent definitions and lifecycle intent](agents.md#postgresql-owns-agent-definitions-and-lifecycle-intent)). Bundled PostgreSQL is the default deployment mode, and an externally provisioned compatible database is supported through the same persistence contract ([Bundled PostgreSQL by default with external PostgreSQL support](packaging.md#bundled-postgresql-by-default-with-external-postgresql-support)).

Agent working files live in a persistent workspace that survives sandbox replacement and idle shutdown. On Kubernetes it is one workspace PVC per agent; under the `kvm` runtime it is attached to the guest as a virtio-blk disk, and it also holds the agent's recovery journal ([In-house VMM as the agent sandbox](sandbox.md#in-house-vmm-as-the-agent-sandbox)). A sandbox refuses a workspace volume the other runtime laid out, and the agent's status says so; there is no conversion.

An agent's memory remains logically owned by that agent ([Unified memory per agent](#unified-memory-per-agent)). Sharing a database creates no shared memory between agents and grants no additional access. The database is shared within one installation ([Installation and tenancy](identity.md#installation-and-tenancy)). Agents reach structured state only through authenticated orchestrator operations, never with direct database access ([Agent state access through the orchestrator](agents.md#agent-state-access-through-the-orchestrator)).

Shared skill bundles, attachments and published artifacts live on an orchestrator-managed persistent volume behind an authenticated API ([Orchestrator-managed file storage on a persistent volume](#orchestrator-managed-file-storage-on-a-persistent-volume)); PostgreSQL stores their metadata and domain references, and agent workspaces stay separate.

Usage: while an agent sleeps, the console reads its retained structured history and the orchestrator records new deliveries. On wake, execution loads the required state and reattaches the agent's persistent workspace. Browsing workspace files while the agent sleeps needs a separate access design; choosing PostgreSQL does not solve that filesystem operation.

Installation requires PostgreSQL and persistent workspace storage. Database backups alone cannot restore workspace files. Recovery must account for both, and a database transaction does not atomically commit filesystem changes or external tool effects. Disposable caches may be added for demonstrated needs, but no additional database engine or cache is required.

**Rationale.** One authoritative structured store supports coordination and access while agent sandboxes are absent, without synchronizing authoritative histories across per-agent databases. PostgreSQL accommodates a shared control plane with concurrent writers. This choice does not itself establish control-plane replication, database high availability or measured capacity.

Alternatives considered: central SQLite behind one active control-plane instance would simplify deployment but constrain concurrent writers and complicate a later high-availability design. Central coordination storage plus authoritative SQLite per agent would distribute ownership physically but add cross-store coordination, backup and sleeping-agent access work. Neither was selected.

**What changed.** The original described the workspace as surviving "pod replacement" in an agent pod. [In-house VMM as the agent sandbox](sandbox.md#in-house-vmm-as-the-agent-sandbox) made it a per-agent PVC attached to the sandbox (a guest disk under `kvm`), holding the workspace and recovery journal, and laid out per runtime. Retention, listed as open, was settled by [Retain durable data until explicitly deleted](#retain-durable-data-until-explicitly-deleted) (keep until explicit deletion). Shared-file storage mechanics, listed as open, were settled by [Orchestrator-managed file storage on a persistent volume](#orchestrator-managed-file-storage-on-a-persistent-volume); database deployment mode by [Bundled PostgreSQL by default with external PostgreSQL support](packaging.md#bundled-postgresql-by-default-with-external-postgresql-support).

**Open details.** Persistence module interfaces and schema; large-payload placement; connection pooling; database deployment mechanics and availability; workspace storage classes and provisioning; and backup and restore consistency.

**Validation scenarios.** Read conversations while an agent sleeps; queue work while its sandbox is absent; restore an agent's state and files after sandbox replacement; commit concurrent lane updates without lost state; reject unauthorized cross-agent reads; and recover database plus workspace data from a documented backup procedure.

## Orchestrator-managed file storage on a persistent volume

Accepted 2026-10-23.

**Decision.** Store shared file bytes (shared skill bundles, attachments and published agent outputs) on a persistent volume managed by the orchestrator, and expose access through an authenticated API that agents and human clients both use. The user selected this model on 2026-10-23, describing it as an internal object-storage facility intended to reduce dependencies. No external S3-compatible service is required. The interface is a native Cubebot API without S3 wire compatibility ([Native Cubebot storage API](#native-cubebot-storage-api)).

Ownership: a storage module inside the existing Rust orchestrator owns transfers, file publication, integrity information and physical cleanup. PostgreSQL remains authoritative for metadata, lifecycle records and relationships to domain resources. Domain authorization decides who may read, publish or remove a file. The persistent volume stores bytes; it is not mounted into agent sandboxes for direct shared access.

Usage: an agent produces a report in its own persistent workspace, uploads it through the orchestrator and attaches the returned artifact reference to an authorized task or room. A user can later download that published report while the agent sleeps. Skill imports and revisions use the same storage module for bundles, with their lifecycle controlled by the skills module.

Agent workspaces stay separate ([Durable storage](#durable-storage)). This facility stores shared published content; it does not replace local workspace filesystem operations or automatically synchronize every agent file. Downloading an object does not confer access to unrelated files or bypass the resource's permissions.

Durability requirements: incomplete uploads must not appear as completed artifacts. Acknowledged publication requires the file and metadata to satisfy a documented durability contract. Recovery must reconcile interrupted publication, lost responses, stale uploads and unreferenced content. A database transaction cannot atomically commit filesystem changes. Physical deletion must respect live references and any retained revisions or backup requirements.

Capacity requirements: bound transfer memory, concurrent streams, staging space, final storage use and background maintenance. Large transfers must not monopolize the orchestrator's control work. Storage exhaustion, integrity failure and unavailable volumes must produce explicit outcomes. Representative load and recovery tests are required before claiming support for a given agent count or throughput.

Availability follows [One active orchestrator with automatic restart](agents.md#one-active-orchestrator-with-automatic-restart): one active orchestrator with restart recovery and accepted downtime. The deployment must preserve exclusive ownership of the mounted storage during replacement. Kubernetes volume access mode alone is not a substitute for the complete active-owner contract. Recovery depends on storage-driver and filesystem behavior, which need qualification.

Backups must cover PostgreSQL, this shared-file volume and agent workspaces with a consistent restore procedure. [Cubebot-coordinated backup and restore workflows](packaging.md#cubebot-coordinated-backup-and-restore-workflows) assigns workflow coordination and restore validation to Cubebot. A persistent volume is not by itself a backup or a guarantee of replication. File cleanup must account for the database restore points the installation intends to keep.

**Rationale.** Requiring S3-compatible storage, or supporting both object storage and filesystem backends initially, was considered. The volume-backed module reduces required services and keeps one storage implementation to qualify. Its API hides layout from consumers, leaving room for later changes based on measured needs.

**Open details.** Native transport and endpoint contracts; transfer and resume contract; volume requirements; publication and cleanup mechanics; access revocation during downloads; quotas; retention; backup tooling; and recovery and throughput targets.

**Validation scenarios.** Agent upload and authorized human download; sleeping-agent artifact access; rejected cross-scope access; interruption at each publication boundary; lost acknowledgement and retry; full disk and database outage; shared reference deletion; orchestrator replacement; and restoring matching metadata and bytes.

## Native Cubebot storage API

Accepted 2026-10-23.

**Decision.** Expose shared file storage ([Orchestrator-managed file storage on a persistent volume](#orchestrator-managed-file-storage-on-a-persistent-volume)) through a native Cubebot API and corresponding native agent tools. The user selected this model on 2026-10-23. S3 wire compatibility and a separate S3 credential model are outside the initial interface. Exact transport, endpoints and schemas are implementation-design details.

Usage: an agent uploads a report from its workspace, receives a stable artifact identity and attaches it to an authorized task or room. A human downloads it through the console under the same resource-access rules. Skills use the same storage module for package bytes while their domain module owns revision publication and installation.

Ownership: the storage module owns transfer state, integrity information, file publication and reclamation. The API adapter authenticates callers and translates requests into that interface. Domain modules own resource references and access decisions. Agents use their existing authenticated orchestrator identity; human clients use platform authentication. Storage introduces no independent set of user accounts and grants no broad volume access.

The interface must cover bounded uploads, completion and status lookup, metadata inspection, authorized download, and lifecycle operations appropriate to the caller. Stable operation identities reconcile lost responses and safe retries. Partial transfer, stored bytes and successful attachment to a resource must remain distinguishable outcomes. A client must not infer successful publication merely because it finished sending bytes.

Consumers address logical artifacts and resource references, never arbitrary server paths. Shared-volume layout and PostgreSQL schema stay implementation details. Agent tools use the same underlying domain operations as human clients rather than maintaining a second file store. An object identifier is not an access credential.

The native interface keeps every durability, authorization, capacity and backup requirement of the shared-file storage decision; a smaller protocol does not weaken them.

**Rationale.** An S3-compatible interface for existing S3 clients was considered. It would add protocol and authentication semantics beyond the selected Cubebot workflows. The native API serves agents and the console directly with one identity and permission model and no external storage dependency.

**Open details.** Transport and endpoint design; transfer limits and resumability; range reads; resource attachment contracts; lifecycle operation scopes; revocation during streaming; and error and retry schemas.

**Validation scenarios.** Agent upload and console download through the same artifact record; resource-scoped access denial; idempotent completion after lost acknowledgement; explicit incomplete-upload status; rejection of caller-supplied filesystem paths; and unchanged stored content across orchestrator restart.

## Retain durable data until explicitly deleted

Accepted 2026-10-24.

**Decision.** Retain durable Cubebot data until an authorized explicit deletion removes it. Do not enable automatic age-based expiry by default. Retiring an agent preserves its records and workspace. The user selected this default on 2026-10-24.

Scope includes captured conversation and tool-result history, task records and results, durable memories, published artifacts, shared skill records and bundles, and retained agent workspaces. A screenshot the agent takes through its computer-use tool is that tool call's result, committed to the turn ledger and kept and deleted with the conversation; frames of a person's desktop session are never stored ([Desktop as a VMM device](sandbox.md#desktop-as-a-vmm-device)). Optional retention rules must be explicitly configured with a defined scope before they can remove durable data; this decision selects no expiry periods or enabled retention rules.

Usage: a person returns to a retired research agent months later and can inspect its retained conversation, work results and memory, subject to current access permissions. Dreaming and context compaction may have improved derived knowledge, but age, inactivity, retirement and completed tasks have not erased the underlying retained records.

Ownership: domain modules authorize deletion and identify affected records and references. Storage owns physical reclamation after the deletion contract permits it. Memory and history enforce provenance and suppression rules. Agent lifecycle releases execution resources without treating retirement as a data-deletion instruction ([Persistent and task-bound created agents](agents.md#persistent-and-task-bound-created-agents)). Backup management owns backup-copy retention and recovery behavior ([Cubebot-coordinated backup and restore workflows](packaging.md#cubebot-coordinated-backup-and-restore-workflows), [Backups without pauses](packaging.md#backups-without-pauses)).

This is a retention default, not an immutable filesystem or a requirement to keep every historical file version. Ordinary authorized workspace edits, replacements and explicit file removals remain possible. Memory correction and consolidation stay governed by the memory sections above, from [Preserve source history during memory consolidation](#preserve-source-history-during-memory-consolidation) through [Revise human-corrected memories from new evidence](#revise-human-corrected-memories-from-new-evidence). Temporary staging, caches and diagnostic logs need separate bounded cleanup policies; retained tool-result evidence and published outputs must not be reclassified as temporary merely to reclaim space.

Storage capacity is finite. Apply quotas, show capacity pressure, and reject or pause affected writes or work when they cannot be stored safely. Do not silently delete retained history, memories, artifacts or retired workspaces to admit new data. Bounded capture and redaction still apply before persistence; the interface must represent omitted or truncated output honestly.

Explicit deletion must have clear scope. Removing a memory from retrieval, deleting its source conversation, deleting an artifact and deleting all of an agent's data are different operations. The specification must define treatment of derived memories, embeddings, active contexts, references and later extraction for each. A missing source must be shown as unavailable rather than cited as retrievable evidence. Existing correction and suppression protections must survive stale writes and background processing.

Deletion in the active installation does not by itself establish erasure from backups or external systems. Document backup retention, restoration behavior and any deletion records needed to prevent unintended reappearance during recovery. Do not promise immediate physical erasure of every copy or deletion of externally published material through a local deletion operation.

**Rationale.** Enabling configurable age-based expiry by default, with separate source and derived-memory lifetimes, was considered. The user selected preservation until explicit deletion to support persistent agents and inspectable evidence.

**Open details.** Deletion permissions and scope; derived-data propagation and suppression; workspace access for retired agents; backup-copy handling; optional policy interfaces; quotas and capacity-pressure behavior; physical reclamation; and temporary-data cleanup periods. These mechanics will be proposed in the consolidated specification.

**Validation scenarios.** Retain old conversations and retired workspaces; consolidate without source deletion; enforce current access to retained records; report capacity pressure without silent eviction; distinguish cache cleanup from durable-data removal; perform scoped authorized deletion; reconcile concurrent extraction and references; and restore without misrepresenting deleted data or backup-copy erasure. Implementation remains pending.

## One knowledge base for the installation, in Markdown

Accepted 2026-11-04. The user asked for a Notion-inspired wiki that people and agents both write, global by default, with a per-agent capability to turn it off, and asked for the format to be researched.

**Decision.** The installation has one knowledge base: a tree of pages and folders (`kb_pages`, migrations/0048_knowledge_base.sql). Every active person reads and writes all of it without a grant; every agent does while its `knowledge_base` capability is on, which is the default, and the orchestrator checks that capability on each runtime call, not only when it offers the tools.

A page's body is **Markdown** (GitHub-flavored), and that text is the stored truth. Agents read and edit it as text through `kb_read`, `kb_edit`, `kb_write` and `kb_create`; the console edits the same text as blocks with TipTap and its Markdown extension. Diagrams are ` ```mermaid ` fences, which the console draws and an agent writes like any code. Images and attachments are blobs on the storage volume, linked from the body by URL.

Outside images in a page load when it opens, without a referrer, by the user's decision on 2026-11-05; an agent writing a page can therefore make a reader's browser request a URL it chose. Chat keeps them as links. An image that fails to load shows a link to the original.

Concurrent writers are handled by revision: a whole-body save names the revision it read and is refused when the page moved on, and the console then asks the person which version to keep. An agent's `kb_edit` replaces one exact passage, so it needs no revision and cannot overwrite what it did not read. Opening a page never saves it, so what an agent wrote stays byte for byte until someone changes it.

Deleting moves a page and everything under it to the trash; anyone can restore it. Only a person can delete from the trash for good, so an agent's mistake is always recoverable. One author's saves within five minutes share one row of version history.

**Rationale.** The format is what both audiences touch, and the two pull apart. Block JSON, as Notion, BlockNote and raw ProseMirror documents store it, is exact for a block editor but expensive and error-prone for a model: every edit has to reproduce a nested schema, and search, diffs and history all need a renderer. Markdown is what models read and write best, is greppable by PostgreSQL full-text search as it stands, diffs line by line, and outlives any editor. The cost is that the editor can only offer what Markdown can say: table cells hold one paragraph, and there are no merged cells or columns. That was accepted, and those limits are enforced in the editor's schema so nothing is silently lost on save.

TipTap was chosen over BlockNote and Lexical because its core and Markdown extension are MIT-licensed and maintained together, tables and task lists round-trip, and its block handle, slash menu and bubble menu are small to build on the house controls so the page keeps the console's design. Its drag-handle extension requires collaboration packages as peers, so the block handle is a few lines on ProseMirror's own drag support. Mermaid was chosen for diagrams over a canvas format such as Excalidraw because an agent can write it and a text diff can show it.

Real-time co-editing (CRDT, such as Yjs) was considered and left out: it would make a binary document the source of truth, or need a server-side bridge to Markdown, for a case the revision check handles honestly. An open page polls every 5 seconds, so another author's save appears while nobody is typing there.

**Scope.** No per-page permissions, comments, databases, public links or backlinks. Without `CUBEBOT_STORAGE_DIRECTORY` pages work and attachments are refused.
