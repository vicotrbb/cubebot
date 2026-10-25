# ADR 0005: Agent execution lanes

Status: Accepted

Date: 2026-10-22

Agents have unified memory under [ADR 0004](0004-unified-agent-memory.md) and can receive work through chat, rooms, messaging, and tasks. Execution must define where turns serialize and where work can proceed concurrently.

Decision: Adopt the OpenBot-style hybrid execution model described in the architecture discussion. The user selected this model on 2026-10-22.

- Each agent has one main lane shared by normal chat, room work, and external messaging. A lane is a durable queue that admits one active turn at a time; it is not a shared conversation transcript.
- Each task has its own lane and conversation context. A task lane admits one active turn at a time and can run beside the main lane and other task lanes.
- The default task concurrency limit is two per agent, configurable from one to four. The main lane is outside that task count. Thus the default allows one main turn plus two task turns, subject to provider capabilities and installation capacity.
- Provider and account concurrency restrictions can reduce execution to one turn at a time. Under [ADR 0006](0006-native-rust-agent-engine.md), Cubebot enforces these constraints around model inference; it does not delegate execution to an external provider runtime.
- Installation-wide admission also limits aggregate concurrent execution. When capacity is scarce, eligible queued human messages take priority, followed by queue age. The numerical installation limit and capacity configuration remain open.
- Different agents execute independently subject to shared capacity limits. Room work occupies the assigned agent's main lane, not every agent in the installation.

Usage: While an agent executes a room turn, its normal chat waits in the main lane. Two task lanes may still execute. A third task waits for capacity. If its provider only permits one turn, every other lane waits instead.

All lanes retain the same agent identity, memory ownership, and workspace. They do not become separate agents or acquire separate pods merely because they are concurrent. Consistent with the reference model, there is no automatic per-task filesystem isolation. [ADR 0033](0033-separate-runtime-and-tool-containers.md) separates runtime and tool execution into containers within that shared agent pod, without creating per-lane containers. Shared workspace writes require explicit coordination. Task sessions do not concurrently control the agent's shared desktop; [ADR 0037](0037-main-and-task-browser-sessions.md) assigns them separate browser sessions with independent action targeting.

Memory mutations follow the reference model's per-turn staging and successful-completion commit, with conflict checks. This does not roll back filesystem changes or external actions from a failed turn. Durable staging, crash reconciliation, and interaction with dreaming remain implementation design questions.

Alternatives considered: One active turn for the entire agent simplifies shared-state ownership but prevents parallel task execution. Arbitrary concurrent turns without lane serialization expose more ordering and coordination problems. The selected hybrid preserves ordered execution within each lane and bounded parallelism across tasks.

Ownership: Scheduling owns lane admission and concurrency limits. Provider adapters report execution capabilities. Memory owns mutation validation and commit. Tool and workspace coordination must have explicit ownership in the subsequent module design. A lane remains reserved while a start outcome is uncertain until recovery resolves it; a timeout alone is not permission to duplicate execution.

Related browser decisions: [ADR 0036](0036-structured-and-visual-browser-interaction.md) supports structured and visual actions in the same browser session. [ADR 0037](0037-main-and-task-browser-sessions.md) assigns a main session and separate task sessions, subject to capacity limits.

Open details: Scheduling fairness under sustained human traffic; lane yielding and interruption mechanics; installation capacity; workspace and browser coordination; durable recovery; and background memory maintenance. [ADR 0071](0071-queued-messages-and-explicit-steering.md) selects queued ordinary messages and explicit steering at safe execution boundaries. ADR 0016 defines Cubebot messaging and delegation semantics; external provider-native child agents remain outside the native engine architecture selected in ADR 0006.

Validation scenarios: Main turn plus two tasks; excess task queued; sequential turns within one lane; provider-enforced serialization; unrelated agent progress during room work; concurrent memory edits; conflicting workspace operations; uncertain provider start; and recovery without duplicate task execution.

Reference: OpenBot revision `cfee2971f29cdb7c16272e60ae912de7fbb30693`, inspected in the [runtime research](../research/openbot-agent-runtime-memory.md) and [collaboration research](../research/openbot-collaboration-channels.md). This ADR adopts the described execution design, not a blanket commitment to every upstream implementation detail. Cubebot implementation and runtime validation are pending.
