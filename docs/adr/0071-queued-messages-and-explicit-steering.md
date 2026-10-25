# ADR 0071: Queued messages with explicit steering

Status: Accepted

Date: 2026-10-23

Decision: Queue ordinary messages for their next eligible turn. Provide an explicit action to steer selected active work at its next safe execution boundary. Stopping work is a separate explicit action. The user selected this behavior on 2026-10-23.

Usage: An agent researches suppliers while a person sends another ordinary message; the message waits under the existing lane scheduling rules. To change the ongoing research to Brazilian suppliers, the person chooses the active assignment and submits a steering instruction. The runtime incorporates it before continuing with subsequent affected actions and exposes acknowledgement in the conversation.

Ordinary messages retain the routing and execution semantics in ADRs 0005 and 0014–0016. In a room, mentions or the coordinator select assignments as before. A new message does not silently become a correction to an existing assignment. Queuing does not require every concurrent task of the agent to finish before an eligible lane can run.

Steering targets an explicit execution or assignment identity. An agent with multiple active lanes must not receive an ambiguous agent-wide correction. A room steering action must identify its intended assignment; it does not implicitly broadcast a change to every member or delegated task. Propagation across delegated work remains a separate design detail.

Ownership: The orchestrator authorizes and durably records steering against the target work. Scheduling and the runtime coordinate delivery with that work's lifecycle. The runtime incorporates eligible instructions at a safe boundary without creating a second concurrent turn in the same lane. The frontend presents the target and delivery state. Steering does not grant new tools, credentials, resource access, or budget.

Distinguish a submitted request, orchestrator acceptance, and runtime incorporation. Incorporation means the instruction entered the active execution context; it is not proof that the model completed the requested change correctly. Retain instruction provenance and ordering in durable history, and protect delivery against duplicate requests and reconnect retries.

Safe-boundary handling must preserve model and tool protocol integrity. Before starting a subsequent affected action, the runtime checks for accepted steering that has reached it. A previously issued external action can still finish and cannot be undone by steering. Exact provider-stream interruption and tool cancellation mechanics remain engineering work. The interface must not claim immediate incorporation while an operation is still running or the agent is disconnected.

If work finishes before a steering instruction can be incorporated, show that outcome explicitly. Do not silently reopen completed work or convert the correction into a new assignment. A user can submit a follow-up through the ordinary message path. Paused, cancelled, or failed work must not restart merely because steering arrived; their continuation rules still apply.

Alternatives considered: Automatically treating new messages as steering makes ordinary follow-ups ambiguous during long-running work. Always queuing messages requires stopping work to correct its direction. The selected model keeps ordinary sends predictable and allows deliberate corrections without discarding progress.

Related decisions: ADR 0012 bounds disconnected execution; ADR 0053 defines online operations and local drafts; ADR 0068 defines budget pauses; ADR 0070 defines the conversation-first console. [ADR 0072](0072-cancellation-of-delegated-work.md) defines cancellation of selected work and its descendants.

Open details: Steering permission mapping; ordering against completion and cancellation; acknowledgement schema; safe boundaries for each provider and tool; handling multiple corrections; delegated-work propagation; and exact UI labels.

Validation scenarios: Queue a normal follow-up; steer one of several active tasks; preserve room routing; reject unauthorized steering; reconcile a repeated submission without applying it twice; handle completion racing delivery; retain instructions across restart; show pending incorporation during disconnection; and preserve evidence of already-issued external actions. Implementation remains pending.
