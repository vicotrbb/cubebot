# ADR 0067: Shared goal budgets with optional cost limits

Status: Accepted

Date: 2026-10-23

Delegated work can involve many agents and nested tasks. Any configured goal limit must account for the whole collaboration without turning monetary configuration into a prerequisite for ordinary use.

Decision: A goal and its delegated work share one aggregate execution budget. Child tasks and agents may have tighter limits, but delegation cannot multiply or reset the goal's configured allowance. The user selected this model on 2026-10-23 and emphasized that monetary cost is not a primary concern: cost limits are optional configuration, unset by default.

A goal here means the root work request and the tasks and continuations performing that request. Exact domain naming remains open. Users can start work without entering a spending allowance, and Cubebot does not invent a hidden monetary ceiling. Existing installation, provider, resource, concurrency, and authorization constraints continue to apply. An unset monetary limit does not remove finite disconnected-execution authority under ADR 0012.

Usage: A research goal delegates collection, analysis, and document production to several agents. If the goal has a configured cost or other supported aggregate limit, all attributable child work consumes it. If no cost limit is configured, work proceeds without a monetary admission gate while still respecting permissions and available execution capacity.

Ownership: The task module preserves root-goal attribution through delegation, transfer, retries, and continuation. An execution-budget module within the orchestrator owns configured limits, reservations, and reconciled usage. Runtime and tool adapters report actual or estimated consumption with identifiable operations. Cost observations are distinct from enforced limits, and unknown usage must not be presented as zero.

For enabled aggregate limits, allocate bounded allowances before distributed work starts and reconcile consumption afterward. Concurrent allocations must fit within the remaining allowance. A child allocation comes from the parent's remaining allowance; it is not an extra allowance. Exact accounting and reservation mechanics remain to be specified. With monetary limits disabled, no artificial cost allocation is needed solely to authorize work.

Agents call providers directly under ADR 0031. Accounting must distinguish reservations, reported usage, estimates, and unresolved calls. Lost responses and delayed usage must not cause duplicate charging in Cubebot's ledger or premature reuse of an outstanding reservation. A runtime restart or task transfer does not reset consumption. Billing accuracy and enforceable cost bounds depend on provider capabilities; the product must state estimation limits rather than promise an exact external invoice ceiling.

Disconnected work respects existing authority and any already allocated bounded allowance for enabled limits. It cannot obtain more allowance from an unavailable orchestrator. Reassignment must account for an old executor that may still act; expiration alone does not prove an in-flight request consumed nothing.

Agents cannot evade a configured root limit by creating another agent, issuing a direct request, or scheduling a continuation of the same goal. Distinguishing a continuation from an independently authorized new goal must be explicit. Recurring routines and background maintenance need their own attribution rules; this decision does not charge unrelated work to whichever goal last used the agent.

Limit changes require applicable management authority and a visible record. Autonomous delegation does not confer authority to increase its own allowance. [ADR 0068](0068-resumable-pause-on-budget-exhaustion.md) selects a resumable pause with preserved progress when a configured limit is exhausted. Exact supported budget dimensions and user-facing controls remain open. No default token, wall-time, or monetary values are selected here.

Alternatives considered: Independent allowances for each delegated task, bounded only by broader installation limits. The shared model makes a configured goal limit meaningful across collaboration. Mandatory cost budgeting was explicitly excluded by the user's clarification.

Open details: Supported dimensions; configuration and inheritance; attribution for routines and maintenance; reservation sizes; provider estimation; unresolved usage reconciliation; limit-management authority; and pause and resume mechanics.

Validation scenarios: Start work without a cost limit; share an enabled limit across parallel delegates; apply a tighter child limit; preserve consumption through transfer and retry; reconcile duplicate reports; avoid recycling uncertain reservations; and prevent child creation or scheduled continuation from resetting a configured allowance.
