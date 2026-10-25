# ADR 0007: Agent idle lifecycle

Status: Accepted

Date: 2026-10-22

Cubebot gives each running agent a dedicated pod and aims to support thousands of agents. Idle execution policy must balance resource consumption, response latency, persistent identity, and background memory maintenance.

Decision: Make idle lifecycle configurable per agent. The user selected this model on 2026-10-22. Agents sleep by default after a safe idle period. Managers can configure selected agents to remain running. The idle timeout and installation resource limits remain to be specified.

Sleeping stops the agent's pod after preserving durable state. It preserves identity, configuration, conversations, memory, task state, and persistent workspace data. Waking recreates execution from that state. It does not promise preservation of arbitrary in-memory processes, open terminal jobs, or live browser sessions.

Messages, scheduled work, and accepted external events can request a wake. [ADR 0046](0046-scheduled-and-event-triggered-work.md) selects both schedules and external events as sources of durable work. Dreaming is scheduled, budgeted work and can wake a sleeping agent for maintenance. Under [ADR 0025](0025-idle-dreaming-and-visible-results.md), dreaming runs only when all execution lanes are idle and yields safely to arriving work. Its status and results are visible in the console. Being idle does not require continuous inference. Always-running means maintaining the execution environment, not continuously calling a model or bypassing capacity limits.

Sleep eligibility applies to the entire agent, including all execution lanes from [ADR 0005](0005-agent-execution-lanes.md). A quiet main lane is insufficient if another task is active. A normal idle shutdown must not discard active work, pending durable writes, or unresolved action outcomes. The treatment of long-running background tools and safe checkpoints will be specified with execution recovery.

Ownership: The orchestrator retains wake requests while the agent is absent and coordinates pod lifecycle. The native engine reports execution state and completes the required checkpoint or drain before an ordinary idle shutdown. The durable storage design must support access to retained state independently of whether the agent pod is running.

Alternatives considered: Always running for all agents minimizes startup delays but keeps idle pods allocated. Sleeping for all agents reduces idle resource use but cannot accommodate agents requiring a continuously available execution environment. Per-agent policy supports both with a shared lifecycle mechanism.

Consequences: Startup latency becomes part of the user experience for sleeping agents. The console must distinguish sleeping, starting, working, and unavailable states. An always-running policy expresses desired availability, not a guarantee against node failures or capacity shortages. Persistent workspace storage may continue consuming resources while a pod sleeps.

Related decision: [ADR 0018](0018-created-agent-lifetime.md) separates lifetime from idle policy. A task-bound agent can sleep while eligible for work; once retired, it cannot wake for work or scheduled dreaming under this idle policy.

Open details: Idle timeout; wake latency targets; startup admission and retry; wake coalescing; sleep/wake races; background process handling; dreaming cadence and budgets; checkpoint format; and Kubernetes reconciliation mechanics.

Validation scenarios: A message wakes a sleeping agent; a scheduled memory-maintenance job wakes it; concurrent wake requests do not create competing executors; an active task prevents idle shutdown; a message arriving during shutdown is retained and processed; always-running agents remain desired when idle; and startup failure preserves queued work.
