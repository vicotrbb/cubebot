# ADR 0024: Rolling context compaction

Status: Accepted

Date: 2026-10-23

An execution lane can accumulate more conversation and tool output than its model can accept. Preserving durable history does not require sending all of that history with every model request.

Decision: Automatically compact older exchanges into a rolling summary while retaining recent messages and tool interactions. Continue the same execution lane with the smaller context. The user selected this approach on 2026-10-23, following a recommendation for a structured summary that preserves the objective, constraints, decisions, unresolved work, and evidence references.

Compaction runs before the context limit at a safe point between model and tool interactions. Context budgeting must account for instructions, tool definitions, retained exchanges, summary, and response capacity. Exact thresholds and retained-message budgets remain open. Compaction must preserve valid tool-call/result relationships and must not cause completed actions to run again.

Usage: During a long implementation task, older investigations and resolved attempts are summarized. Recent exchanges remain available in detail. The agent continues with the same task identity, pending work, and execution permissions. If an omitted detail becomes necessary, the agent can explicitly retrieve its source through an authorized tool.

Source history remains intact under [ADR 0020](0020-memory-consolidation-and-source-history.md). The summary is derived context with references to the covered history. It is not a replacement for evidence, an automatic long-term memory write, or authority to change task status or grants.

Compaction uses the lane's existing conversation material and previous summary. It does not automatically search the saved knowledge pool or import another lane's context. Explicit recall remains governed by [ADR 0023](0023-tool-initiated-memory-retrieval.md). Automatically maintaining current context and explicitly retrieving long-term memory are separate operations.

Ownership: The native engine's context module owns budget assessment, compaction, and context assembly for each lane. History and memory modules retain their storage responsibilities under ADR 0020. Durable compaction state goes through the orchestrator; disconnected continuation remains subject to ADR 0012 and its local journal limits. Compaction does not require another deployed process.

Implementation requirements: Prepare and validate a replacement before switching the lane to it. A failed or interrupted compaction must leave the previous valid context recoverable. If no valid context fits the model budget, pause or report the failure rather than silently discard required instructions or repeatedly send oversized requests. The detailed recovery protocol remains to be designed.

Alternative considered: Rebuild context around a structured task handoff after each compaction. That makes continuation explicit but discards more conversational continuity. Rolling compaction retains recent exchanges alongside the summary and fits ongoing conversations as well as autonomous tasks.

Consequences: Compaction consumes inference time and budget. Repeated summaries can omit or distort details; source preservation enables recovery but does not guarantee that the agent notices an omission. Summary fidelity and continuation quality need evaluation across multiple compaction cycles.

Related decision: [ADR 0029](0029-model-default-and-purpose-overrides.md) uses the agent's default model for compaction unless a purpose override is configured. Context budgeting must account for both the compaction model and the model continuing the work.

Open details: Token accounting across providers; trigger thresholds; summary schema and model qualification; recent-context allocation; oversized individual results; timeout and retry policy; summary versioning and recovery; and latency, cost, and fidelity targets.

Validation scenarios: Continue a task through repeated compactions; retain a corrected requirement and unfinished work; preserve valid tool interactions; keep concurrent lanes separate; retain source references; avoid implicit memory search; recover after interrupted compaction without repeating an external action; and stop safely when a valid context cannot fit.
