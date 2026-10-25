# ADR 0018: Persistent and task-bound created agents

Status: Accepted

Date: 2026-10-23

[ADR 0017](0017-agent-initiated-creation.md) permits authorized agents to create other agents. Dynamic teams need both reusable specialists and workers whose availability ends with the work that justified their creation.

Decision: Support persistent and task-bound lifetimes, selected at creation within the creation grant. The user selected this model on 2026-10-23. Task-bound workers are the default for agent-initiated creation; persistent creation requires the grant to permit it. Human-initiated creation defaults are not changed by this decision.

Persistent agents remain available for future work until explicitly retired by an authorized actor. Task-bound agents are associated with an originating task and are automatically retired once that task and its dependent work are settled. Retirement makes an agent ineligible for execution. It does not itself delete memories, conversation history, results, or workspace files.

Lifetime and idle policy are separate. Both kinds use [ADR 0007](0007-agent-idle-lifecycle.md): sleeping agents remain eligible to wake, while retired agents do not. An always-running idle policy cannot extend a task-bound lifetime after retirement. Both kinds remain installation-owned agents with their own identities and memories, using the same engine and permission model.

Usage: A lead creates a persistent specialist for recurring analysis and several task-bound researchers for one investigation, if its grant permits both choices. The researchers can sleep between assignments while their originating work remains active. Once the investigation and its dependent work are settled, they retire and their records remain subject to retention policy. The specialist remains eligible for later assignments.

The orchestrator validates the lifetime against the creation grant and stores the originating task for a task-bound agent. It owns automatic retirement. Retirement must account for active execution, pending results, and bounded disconnected work; a terminal parent record alone is not permission to discard unresolved child execution or recovery evidence.

Rationale: One agent model with an explicit lifetime policy supports lasting teams and temporary swarms without making every created worker permanently available.

Alternatives considered: Making every created agent persistent would simplify lifetime behavior but require explicit cleanup of temporary workers. Making every created agent task-bound would prevent creation of reusable specialists without another lifecycle transition. The user selected both lifetimes within grants.

Consequences: The task system must define when originating and dependent work is settled, including failure and cancellation. The console must distinguish a sleeping worker from a retired one. Retirement can release execution resources while retained records and workspace data continue consuming storage. [ADR 0079](0079-explicit-deletion-for-durable-data.md) retains durable records and workspaces until explicit deletion, with no automatic age-based expiry by default.

Open details: Failure and cancellation settlement; assignments outside the originating task; promotion to persistent lifetime; explicit retirement authority; reactivation; descendant lifetimes; and retention and cleanup mechanics. No automatic promotion or memory transfer is implied.

Validation scenarios: A permitted persistent creation succeeds; a task-only grant rejects persistent creation; omitted lifetime defaults to task-bound for agent creation; a sleeping worker wakes while its task remains active; unsettled dependent execution prevents premature cleanup; retirement stops future scheduling; and retained results remain accessible after retirement.
