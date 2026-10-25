# ADR 0014: Direct mentions and a tool-free room coordinator

Status: Accepted

Date: 2026-10-23

Rooms contain human participants and agents. A shared transcript does not imply that every member should run a turn for every message. Incoming requests need explicit routing and task ownership.

Decision: Route a direct agent mention to that agent without an LLM routing call. Otherwise, use a tool-free LLM coordinator. The user accepted this model on 2026-10-23. The coordinator selects one responsible task owner, who can perform the work or delegate to other agents while retaining responsibility for integrating results.

A direct mention selects an eligible room member, subject to authorization and scheduling. It does not bypass a busy lane, capacity limits, or permission checks. Invalid or unauthorized targets must be reported rather than silently replaced with another agent. [ADR 0015](0015-multiple-room-mentions.md) gives each explicitly mentioned agent its own assignment, without coordinator routing or an automatic lead.

The coordinator is a module inside the orchestrator using the model/provider adapter. It has no dedicated agent pod, personal memory, terminal, browser, or work tools. Its input includes the request, room instructions, eligible member descriptions, bounded room history or summary, and relevant open tasks.

Its structured result selects one agent, continues an existing task, asks a clarifying question, or indicates no action. Rust code validates the result against current room and task state before recording and scheduling it. Model output cannot grant permissions or directly execute work. Stale or invalid routing results must be reconciled or reported without broadcasting the request to all members.

Usage: A request addressed to a researcher enters that researcher's room work through its main lane. An unaddressed request goes to the coordinator, which may choose an engineer as owner. The engineer may delegate analysis to other members and collect their results. Assigned work follows [ADR 0005](0005-agent-execution-lanes.md); routing itself does not create another execution lane for the selected agent.

Rationale: Explicit user intent avoids an unnecessary model call. Unaddressed requests can still be handled naturally, with one accountable owner and selective participation. Routing stays small while the ordinary task system handles substantive planning and delegation.

Alternatives considered: Mention-only participation would leave unaddressed requests inactive. Having every eligible member evaluate every request would increase inference and coordination work. A coordinator that creates a complete multi-agent plan would take on planning and integration responsibilities already needed in the task system.

Consequences: Unaddressed routing consumes inference and requires an available coordinator model. Receipt of a message, successful routing, and execution are separate durable states. The room transcript remains available as context without waking all members. Membership changes during routing require validation before assignment.

Related decision: [ADR 0016](0016-agent-messaging-and-delegation.md) makes ordinary agent output passive. Agents initiate work through explicit collaboration operations, and expected results resume waiting work through durable relationships. Human room requests retain the routing rules above.

Open details: Coordinator model and credential selection; retry limits and cost budgets; human reply and continuation behavior; detailed delegation contracts; room concurrency; and loop prevention. No heuristic bypass for unmentioned human requests is adopted by this decision.

Validation scenarios: One valid mention bypasses LLM routing; unaddressed work selects one owner; clarification and no-action results; unavailable or malformed coordinator output; revoked membership during routing; a busy selected agent queues work; and the owner delegates while remaining responsible for integration.
