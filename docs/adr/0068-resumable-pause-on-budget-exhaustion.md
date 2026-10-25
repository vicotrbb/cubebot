# ADR 0068: Preserve progress and pause on configured budget exhaustion

Status: Accepted

Date: 2026-10-23

Configured execution limits can be reached while a goal and its delegated tasks are incomplete. The system must respect those limits without discarding useful progress or treating a recoverable limit as successful completion.

Decision: Pause affected work with preserved progress when a configured execution budget is exhausted. The user selected this model on 2026-10-23. Record the reason and notify the responsible authorized user. An authorized limit change can make the work eligible to continue. Monetary limits remain optional and unset by default under ADR 0067.

Usage: A goal reaches an enabled aggregate limit while several specialists are working. Cubebot stops admitting further affected actions, retains their results and continuation state, and shows the goal as paused for that limit. After an authorized user supplies sufficient allowance and requests or permits continuation under the eventual resume contract, work continues from reconciled state rather than starting over.

Ownership: The execution-budget module detects exhausted allowances and stops new allocations. Scheduling blocks affected starts. Runtime and tool modules settle or reconcile in-flight activity and preserve durable progress. The task module owns the resumable state and dependency relationships. Notifications follow ADR 0054's recipient permissions and user preferences.

Exhaustion is scoped to the limit reached. A root-goal limit affects the root and all attributable delegated work. A tighter child-only limit pauses that child and prevents dependent progress where its result is required; it does not exhaust the parent's remaining allowance or block unrelated goals. An installation-wide exhausted limit can affect multiple goals. Exact supported dimensions remain open.

A pause stops new affected actions; it cannot undo already-issued requests or external effects. Distinguish a requested pause, draining or uncertain execution, and confirmed paused state. Preserve completed outputs, pending tool outcomes, and reported or reserved usage. The system must not issue an extra unbudgeted model call merely to summarize why it paused. A deterministic status can explain the limit and link to existing progress.

When agents are disconnected, their existing allocations and finite authority still bound execution. Central exhaustion cannot be assumed instantly visible to them. Accounting must include outstanding allocations and uncertain consumption before allowing further work elsewhere. Restart or transfer cannot clear the pause, reset consumption, or blindly replay an action.

An authorized limit adjustment is necessary when the applicable allowance is insufficient, but it is not enough to bypass other conditions. Resume rechecks permissions, cancellation state, dependencies, provider access, and unresolved execution outcomes. It retains the same goal identity and prior consumption. A separately cancelled goal must not restart because its limit was later increased.

Preserved progress means durable task state, recorded outputs, context checkpoints, workspace files, and recoverable action evidence. It does not promise suspension of arbitrary processes or live browser sessions. Once execution is safely settled, ordinary idle lifecycle rules may allow the agent pod to sleep. Unrelated authorized work may still run within its applicable capacity and limits.

Alternative considered: End the goal with partial results and require a new request. A resumable pause better supports persistent work while making the configured limit effective.

Open details: Limit-state schema; pause acknowledgement and draining; notification aggregation; manual versus automatic eligibility after adjustment; idle lifecycle integration; and recovery of individual tool types.

Validation scenarios: Pause a goal and delegates at an enabled aggregate limit; pause only a child at its tighter limit; retain partial results across restart; reconcile an in-flight side effect; respect offline allocations; resume without duplicate work after adjustment; preserve cancellation; and never pause for an unset monetary limit.
