# ADR 0073: Contain delegated failures and let the parent adapt

Status: Accepted

Date: 2026-10-23

Decision: A failed delegated assignment reports its failure and available partial results to its parent without automatically stopping the whole goal or healthy sibling assignments. The parent may adapt the approach, reassign work, continue with an explicit limitation, or seek human help. The user selected this behavior on 2026-10-23.

Usage: Three researchers contribute to a market analysis. One exhausts an applicable recovery path and reports that a required source cannot be accessed. The other researchers continue. The parent receives the failed outcome and can choose an authorized alternative source or disclose the missing evidence in its final result.

Ownership: The task module records the child's outcome, dependency relationship, partial-result references, and whether execution has settled. Scheduling makes an eligible parent continuation available to inspect the failure. The parent agent chooses the response within its instructions, permissions, and configured limits. Failure handling is ordinary agent execution; it does not require another LLM coordinator.

A failed dependency must not remain pending forever or appear successful merely because the child produced some output. Preserve completed, partial, and uncertain results with their provenance. A parent must assess whether required evidence or deliverables are still missing before claiming success. Continuing with a limitation does not authorize declaring an unmet requirement fulfilled.

Outcome delivery and parent continuation must survive restart and tolerate duplicate notifications. Parent processing remains serialized within its lane. The scheduler may combine pending child outcomes into an eligible continuation, but must preserve each outcome and avoid starting concurrent parent turns. An active parent receives the outcome through the defined runtime boundary; a waiting parent can become eligible before all healthy siblings finish. Exact delivery and batching mechanics remain open.

Recovery retains the same goal ancestry and configured aggregate budgets under ADR 0067. Record replacement attempts and their relationship to the failed work rather than erasing failure history. Retries and reassignments must be bounded against repetitive, unproductive recovery; exact attempt and no-progress rules remain open. If recovery cannot proceed, expose the reason and ask for help or report the limitation.

Provider waiting under ADR 0030, budget pauses under ADR 0068, explicit cancellation under ADR 0072, and uncertain external outcomes remain distinct from settled task failure. A provider outage does not authorize automatic model or connection fallback. A network timeout alone does not prove an external action failed; resolve its outcome before retrying or assigning a duplicate effect to another worker.

Explicit cancellation takes precedence over recovery. A cancelled root cannot resume because a child reports failure. A parent may acknowledge a user-cancelled child and explain the impact, but must not automatically recreate the cancelled work through another agent or task. This decision does not grant authority to undo a user's stop request.

Failure containment does not remove dependencies: work that requires a missing result cannot consume an invented success. Healthy siblings continue unless another applicable condition stops them. The settlement of a failed assignment's own active descendants and tool operations must be explicit before resource release or task-bound agent retirement. This decision does not select a blanket cascade policy for those descendants.

Alternatives considered: Automatically stopping the entire goal makes one failed branch interrupt otherwise useful progress. Selecting a stop-or-recover policy on every delegation adds configuration to ordinary collaboration. The user chose parent-directed adaptation as the default behavior.

Open details: Outcome schema and failure classification; parent wake-up and batching; recovery attempt and no-progress limits; failed-parent descendant settlement; human escalation; and final goal completion criteria.

Validation scenarios: One child fails while siblings progress; a waiting parent receives failure without waiting indefinitely; partial results remain identifiable; duplicate delivery does not duplicate recovery; cancellation blocks replacement work; provider waiting does not become model fallback; uncertain effects prevent blind retries; and recovered work retains goal ancestry and accounting. Implementation remains pending.
