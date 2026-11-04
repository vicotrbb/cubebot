# Rooms and delegation

These decisions cover how work reaches agents in rooms and how agents work together: direct mentions and a tool-free coordinator for human room requests, explicit messaging, assignment and transfer between agents, steering of active work, cancellation, failure containment and goal completion. Execution itself follows [Agent execution lanes](agents.md#agent-execution-lanes) and the sandbox model in [Agent-centric execution in a sandbox boundary](sandbox.md#agent-centric-execution-in-a-sandbox-boundary).

## Direct mentions and a tool-free room coordinator

Accepted 2026-10-23.

**Decision.** Route a direct agent mention to that agent without an LLM routing call. Otherwise, use a tool-free LLM coordinator. The user accepted this model on 2026-10-23. The coordinator selects one responsible task owner, who can perform the work or delegate to other agents while retaining responsibility for integrating results.

A direct mention selects an eligible room member, subject to authorization and scheduling. It does not bypass a busy lane, capacity limits, or permission checks. Invalid or unauthorized targets must be reported rather than silently replaced with another agent. [Independent assignments for multiple room mentions](#independent-assignments-for-multiple-room-mentions) gives each explicitly mentioned agent its own assignment, without coordinator routing or an automatic lead.

The coordinator is a module inside the orchestrator using the model/provider adapter; it is one of the orchestrator's own model duties. It has no dedicated agent sandbox, personal memory, terminal, browser, or work tools. Its input includes the request, room instructions, eligible member descriptions, bounded room history or summary, and relevant open tasks.

Its structured result selects one agent or continues an existing task. Amended 2026-11-03 by the user: a person in a room always gets an answer from the best-placed member, so the coordinator no longer asks clarifying questions itself or settles a message with no action; greetings and chatter go to an owner too, who may ask the question. When the coordinator cannot produce a valid result after its retries, or has no provider connection, the message goes to the room's first member by name rather than failing silently. Rust code validates the result against current room and task state before recording and scheduling it. Model output cannot grant permissions or directly execute work. Stale or invalid routing results must be reconciled or reported without broadcasting the request to all members.

Usage: a request addressed to a researcher enters that researcher's room work through its main lane. An unaddressed request goes to the coordinator, which may choose an engineer as owner. The engineer may delegate analysis to other members and collect their results. Assigned work follows [Agent execution lanes](agents.md#agent-execution-lanes); routing itself does not create another execution lane for the selected agent.

Consequences: unaddressed routing consumes inference and requires an available coordinator model. Receipt of a message, successful routing, and execution are separate durable states. The room transcript remains available as context without waking all members. Membership changes during routing require validation before assignment. Under [Explicit agent messaging and delegation](#explicit-agent-messaging-and-delegation), ordinary agent output is passive: agents initiate work through explicit collaboration operations, and expected results resume waiting work through durable relationships. Human room requests retain the routing rules above.

**Rationale.** Rooms contain human participants and agents. A shared transcript does not imply that every member should run a turn for every message; incoming requests need explicit routing and task ownership. Explicit user intent avoids an unnecessary model call. Unaddressed requests can still be handled naturally, with one accountable owner and selective participation. Routing stays small while the ordinary task system handles substantive planning and delegation.

Alternatives considered: mention-only participation would leave unaddressed requests inactive. Having every eligible member evaluate every request would increase inference and coordination work. A coordinator that creates a complete multi-agent plan would take on planning and integration responsibilities already needed in the task system.

**Open details.** Coordinator model and credential selection; retry limits and cost budgets; human reply and continuation behavior; detailed delegation contracts; room concurrency; and loop prevention. No heuristic bypass for unmentioned human requests is adopted by this decision.

**Validation scenarios.** One valid mention bypasses LLM routing; unaddressed work, chatter included, selects one owner; unavailable or malformed coordinator output; revoked membership during routing; a busy selected agent queues work; and the owner delegates while remaining responsible for integration.

**Amended 2026-11-04 by the user: room work stays in the room.** A delivery is still a turn on the recipient's ledger, in its main lane, with retries and cancellation unchanged, but it belongs to the room. Its request and answer show only in the room; they are not part of the agent's own conversation, its unread count or its working context. A delivery's context is the room's recent messages, and the agent's own conversation never sees it. Memory stays one pool per agent ([Unified memory per agent](memory.md#unified-memory-per-agent)), so what it learned in a room is still available elsewhere. Retrying or stopping a delivery happens in the room, for someone with `conversation.send` on the agent.

## Independent assignments for multiple room mentions

Accepted 2026-10-23.

**Decision.** When a message explicitly addresses more than one agent, each mentioned agent receives its own assignment. The user selected this model on 2026-10-23. Multiple mentions do not invoke the coordinator to choose a lead. Each assignment has one owner and can produce its own response; there is no automatically assigned owner of a combined answer.

Usage: a room request addressed to a researcher and an engineer creates an assignment for each, linked to the same originating message. Both receive the request and relevant room context. Each proceeds when its lane and execution capacity permit. One may finish while the other is still queued or working.

The system preserves the relationship between the originating request, recipient, assignment, and response. Repeated mentions of the same agent in one request do not create duplicate assignments. Retrying delivery of the same accepted request must not duplicate recipient assignments. Permission and membership checks still apply to each intended recipient.

Consequences: responses may overlap or disagree. No implicit dependency, aggregation barrier, or synthesized final answer is created. Users can request coordinated work by addressing one agent and asking it to delegate, or by sending an unaddressed request for coordinator routing. An assignment owner can still delegate its own work under [Explicit agent messaging and delegation](#explicit-agent-messaging-and-delegation). Mentions in ordinary agent output do not invoke this assignment behavior.

Parallel eligibility does not guarantee simultaneous execution. Agent lane serialization and installation capacity from [Agent execution lanes](agents.md#agent-execution-lanes) continue to apply. Cancelling or completing one assignment must not silently settle another.

**Rationale.** Explicit addressing remains predictable and requires no routing inference. A user requesting several independent contributions gets those contributions directly. Alternative considered: invoking the coordinator to select a lead among the mentioned agents would establish one integrated result but introduce routing inference and change the meaning of explicitly addressing several agents. The user chose independent assignments.

**Open details.** Structured mention representation; quoted or incidental names; recipient limits; mixed valid and invalid recipients; and request-wide cancellation.

**Validation scenarios.** Two distinct mentions create two separately owned assignments; repeated mention creates one assignment for that recipient; a request retry creates no duplicates; one busy agent does not require another to wait for its response; and independent results remain attributable to the correct assignment.

## Explicit agent messaging and delegation

Accepted 2026-10-23. Amended 2026-10-27: lane admission is a rule inside the agent loop rather than an orchestrator scheduler.

**Decision.** Adopt OpenBot-style explicit agent messaging and delegation. The user accepted this model on 2026-10-23. Agents initiate collaboration through structured messaging, assignment, or transfer operations. Ordinary progress and final-answer text is recorded in the room without automatically invoking the coordinator or assigning work to mentioned agents. Expected results can resume waiting work through existing task and message relationships.

Room assignment creates a child task with a named recipient, instructions, expected result, and relevant source references. The assigning owner remains responsible for integrating the result. Transfer changes ownership of the whole task and ends the old owner's execution of it. Publishing a result records output for an existing assignment; it is not a new room request.

An owner waiting for delegated work ends its current turn and releases execution capacity. The durable task relationship determines when a continuation becomes eligible. It does not require a model to read the result and rediscover who is waiting. Task completion and result publication must remain distinct enough to handle failures after output was published.

Direct agent messaging outside room assignments uses explicit recipients, a message identity, reply correlation, and whether an answer is expected. Results and informational messages do not request an acknowledgement by default. A correlated result can schedule the requester's continuation without creating a new request to its sender. An explicit new request may continue the conversation under the applicable permissions and budgets.

Following the selected reference behavior, ordinary direct messaging is not a bypass for room delegation: work within a room assignment uses room assignment or transfer operations. A successful direct-request turn can relay its final result if no explicit answer was already sent. Failed or interrupted turns must not be presented as successful results. Automatic relay must be idempotent and must not echo informational or result messages back indefinitely.

Usage: a researcher explicitly assigns implementation to an engineer and yields. The engineer publishes a result and completes its task. Once the required dependencies are satisfied, the researcher resumes to integrate the result and answer the user. A progress sentence naming the engineer creates no additional assignment.

Ownership: the messaging module owns delivery identity and reply correlation. The task module owns assignment, dependencies, transfer, and continuation eligibility. The agent loop's lane rule owns admission to execution lanes ([Agent-centric execution in a sandbox boundary](sandbox.md#agent-centric-execution-in-a-sandbox-boundary)). The transcript stores visible output; displaying text does not itself grant authority to schedule work.

Consequences: agents need tools for collaboration, not just instructions to mention names. Permission checks, duplicate suppression, and bounded execution still apply. Passive output is available as future context even when it starts no new turn. Declining automatic acknowledgements alone is not a complete loop-prevention policy.

Related decisions: permission to collaborate does not itself grant permission to create agents ([Agent creation as a granted capability](agents.md#agent-creation-as-a-granted-capability)). Configured goal limits are shared across delegated work and continuations; monetary limits are optional and unset by default, and delegation does not create additional allowance ([Shared goal budgets with optional cost limits](agents.md#shared-goal-budgets-with-optional-cost-limits)). [Cancel selected work and its descendants](#cancel-selected-work-and-its-descendants) cancels selected work and its descendants, blocks further affected delegation and continuation, and preserves partial results and unrelated work. [Contain delegated failures and let the parent adapt](#contain-delegated-failures-and-let-the-parent-adapt) returns failed child outcomes and partial results to the parent for adaptation while healthy siblings continue. External provider-native child agents remain outside the [native engine](agents.md#native-rust-agent-engine).

**Rationale.** Human room requests follow mention and coordinator routing; agent-authored progress, requests, and results need distinct execution semantics. Alternative considered: routing all agent output like human requests would permit conversational mentions to trigger work but increase routing calls and accidental response chains. The selected model makes actionable intent explicit while preserving autonomous collaboration.

**What changed.** As accepted, an orchestrator scheduler owned admission to execution lanes. Since [Agent-centric execution in a sandbox boundary](sandbox.md#agent-centric-execution-in-a-sandbox-boundary) the orchestrator routes work but does not schedule lanes; lanes are a scheduling rule inside the agent loop.

**Open details.** Exact tool interfaces; outcome delivery and partial-result schema; cancellation settlement and dependency rules; delegation depth and budget accounting; direct-message result batching; timeout behavior; retries; and permitted recipient scope.

**Validation scenarios.** Passive progress containing a mention starts no work; explicit assignment starts only its recipient; transfer leaves one current owner; required results resume a waiting owner; repeated result delivery does not duplicate continuation; fallback direct replies do not duplicate explicit replies; and informational messages produce no automatic reply echo.

## Queued messages with explicit steering

Accepted 2026-10-23. Amended 2026-10-27: the agent's own loop, not an orchestrator scheduler, admits work to lanes and incorporates steering.

**Decision.** Queue ordinary messages for their next eligible turn. Provide an explicit action to steer selected active work at its next safe execution boundary. Stopping work is a separate explicit action. The user selected this behavior on 2026-10-23.

Usage: an agent researches suppliers while a person sends another ordinary message; the message waits under the existing lane scheduling rules. To change the ongoing research to Brazilian suppliers, the person chooses the active assignment and submits a steering instruction. The runtime incorporates it before continuing with subsequent affected actions and exposes acknowledgement in the conversation.

Ordinary messages retain the routing and execution semantics of [Agent execution lanes](agents.md#agent-execution-lanes) and the sections above. In a room, mentions or the coordinator select assignments as before. A new message does not silently become a correction to an existing assignment. Queuing does not require every concurrent task of the agent to finish before an eligible lane can run.

Steering targets an explicit execution or assignment identity. An agent with multiple active lanes must not receive an ambiguous agent-wide correction. A room steering action must identify its intended assignment; it does not implicitly broadcast a change to every member or delegated task.

Ownership: the orchestrator authorizes and durably records steering against the target work. The orchestrator's routing and the agent's runtime coordinate delivery with that work's lifecycle. The runtime incorporates eligible instructions at a safe boundary without creating a second concurrent turn in the same lane. The frontend presents the target and delivery state. Steering does not grant new tools, credentials, resource access, or budget.

Distinguish a submitted request, orchestrator acceptance, and runtime incorporation. Incorporation means the instruction entered the active execution context; it is not proof that the model completed the requested change correctly. Retain instruction provenance and ordering in durable history, and protect delivery against duplicate requests and reconnect retries.

Safe-boundary handling must preserve model and tool protocol integrity. Before starting a subsequent affected action, the runtime checks for accepted steering that has reached it. A previously issued external action can still finish and cannot be undone by steering. Exact provider-stream interruption and tool cancellation mechanics remain engineering work. The interface must not claim immediate incorporation while an operation is still running or the agent is disconnected.

If work finishes before a steering instruction can be incorporated, show that outcome explicitly. Do not silently reopen completed work or convert the correction into a new assignment. A user can submit a follow-up through the ordinary message path. Paused, cancelled, or failed work must not restart merely because steering arrived; their continuation rules still apply.

Related decisions: [Agent-centric execution in a sandbox boundary](sandbox.md#agent-centric-execution-in-a-sandbox-boundary) governs disconnected execution; [Online console operations with local drafts](console.md#online-console-operations-with-local-drafts) defines online operations and local drafts; [Preserve progress and pause on configured budget exhaustion](agents.md#preserve-progress-and-pause-on-configured-budget-exhaustion) defines budget pauses; [Conversation-first workspace](console.md#conversation-first-workspace) defines the conversation-first console; [Cancel selected work and its descendants](#cancel-selected-work-and-its-descendants) defines cancellation of selected work and its descendants.

**Rationale.** Automatically treating new messages as steering makes ordinary follow-ups ambiguous during long-running work. Always queuing messages requires stopping work to correct its direction. The selected model keeps ordinary sends predictable and allows deliberate corrections without discarding progress.

**What changed.** As accepted, scheduling and the runtime coordinated delivery, and disconnected execution was bounded by [Bounded continuation during disconnection](agents.md#bounded-continuation-during-disconnection). Under [Agent-centric execution in a sandbox boundary](sandbox.md#agent-centric-execution-in-a-sandbox-boundary) the orchestrator routes and records steering while the agent's loop schedules its lanes, and a disconnected agent keeps working under its long-lived sandbox authority.

**Open details.** Steering permission mapping; ordering against completion and cancellation; acknowledgement schema; safe boundaries for each provider and tool; handling multiple corrections; delegated-work propagation; and exact UI labels.

**Validation scenarios.** Queue a normal follow-up; steer one of several active tasks; preserve room routing; reject unauthorized steering; reconcile a repeated submission without applying it twice; handle completion racing delivery; retain instructions across restart; show pending incorporation during disconnection; and preserve evidence of already-issued external actions. Implementation remains pending.

## Cancel selected work and its descendants

Accepted 2026-10-23. Amended 2026-10-27: a disconnected agent works under its long-lived sandbox authority rather than a finite per-turn window.

**Decision.** Stopping a goal cancels that goal and all work delegated under it. Stopping an individual child assignment cancels that child and its descendants. Preserve partial results and leave unrelated work running. The user selected this behavior on 2026-10-23.

Usage: a lead delegates a market analysis to three researchers, one of whom delegates a further task. An authorized user stops the root goal. Cubebot blocks further work for that goal, propagates cancellation to every unsettled descendant, and shows stopping until affected execution has settled. Other goals using the same agents remain eligible to run.

Scope follows durable task ancestry, including queued work, waiting continuations, active assignments, and work transferred to another agent. Completed results remain completed evidence; cancelling their parent does not rewrite them as failed or remove their outputs. Informational messages do not create descendants. Goal-attributed requests must retain their ancestry so another collaboration path cannot escape cancellation.

Ownership: the task module authorizes and durably records cancellation scope. Scheduling prevents affected starts and continuations. The agent's runtime and tools stop further affected actions and settle or reconcile in-flight activity. The orchestrator coordinates propagation and recovery. The console distinguishes accepted cancellation, stopping, and settled cancellation; notifications follow existing access and preference rules.

Cancellation and delegation must have a defined ordering. A delegation committed before cancellation is included in its scope. A request to create further work under a cancelled ancestor is rejected. Transfers, retries, late results, steering, budget increases, and orchestrator restarts cannot clear cancellation or recreate the work implicitly. Admission must check effective ancestor cancellation rather than depend solely on asynchronous delivery to every descendant.

Cancellation is scoped to work, not to every activity in an agent's sandbox. Do not stop an entire agent's unrelated lanes merely because one assignment is cancelled. Persistent agents remain available. Task-bound agents follow the retirement rules of [Persistent and task-bound created agents](agents.md#persistent-and-task-bound-created-agents) once originating and dependent work are settled, preserving history and recovery evidence.

Already-issued requests may finish or have an uncertain outcome. Retain their outputs and effect records without treating late arrival as permission to resume the cancelled goal. Cancellation cannot undo external effects. Tool-specific interruption, process cleanup, and recovery must respect unrelated activity and preserve evidence needed to resolve uncertainty.

A disconnected agent keeps working under its long-lived sandbox authority and its local journal ([Agent-centric execution in a sandbox boundary](sandbox.md#agent-centric-execution-in-a-sandbox-boundary)) until it reconnects, its credential is revoked or its sandbox is stopped. The orchestrator cannot claim that recording cancellation instantly stopped it. Track outstanding execution until acknowledgement, revocation or stopping, and reconciliation establish its state. Revoking permission for future work does not prove an external request already issued was rolled back.

Cancelling a child makes its cancelled outcome available to the waiting parent without presenting success or leaving a dependency waiting forever. The parent is outside that child's cancellation scope. [Contain delegated failures and let the parent adapt](#contain-delegated-failures-and-let-the-parent-adapt) allows adaptation to failed dependencies while prohibiting automatic recreation of explicitly cancelled work. Cancelling the root prevents all descendant result arrivals from scheduling further root-goal work.

This decision defines cancellation, distinct from the resumable budget pause in [Preserve progress and pause on configured budget exhaustion](agents.md#preserve-progress-and-pause-on-configured-budget-exhaustion). Recovery retains partial progress, but resuming cancelled work requires a separate explicit continuation contract; no automatic restart is permitted. Data deletion and retention are separate policies.

**Rationale.** Alternative considered: stop only the selected assignment while delegates continue. This preserves independently useful delegate activity but conflicts with the user's selected expectation that stopping a goal also stops acting on its behalf.

**What changed.** As accepted, disconnected agents were subject to the finite per-turn authority of [Bounded continuation during disconnection](agents.md#bounded-continuation-during-disconnection), and outstanding execution was tracked until acknowledgement, expiry or fencing. [Agent-centric execution in a sandbox boundary](sandbox.md#agent-centric-execution-in-a-sandbox-boundary) removed per-turn windows; a disconnected agent can only be stopped by revoking its credential or stopping its sandbox, so those replace expiry. The journaling rules remain.

**Open details.** Cancellation permission mapping; concrete state transitions and aggregation; tool interruption deadlines; disconnected executor fencing; dependency failure handling; explicit continuation of cancelled work; and presentation of unresolved external outcomes.

**Validation scenarios.** Cancel a root with nested delegates; cancel a child while siblings continue; race cancellation with delegation or transfer; cancel queued and waiting work; reconcile late results without resuming the goal; repeat cancellation after restart; retain uncertain external effects; handle disconnected workers; and preserve unrelated lanes and partial outputs. Implementation and runtime qualification remain pending.

## Contain delegated failures and let the parent adapt

Accepted 2026-10-23.

**Decision.** A failed delegated assignment reports its failure and available partial results to its parent without automatically stopping the whole goal or healthy sibling assignments. The parent may adapt the approach, reassign work, continue with an explicit limitation, or seek human help. The user selected this behavior on 2026-10-23.

Usage: three researchers contribute to a market analysis. One exhausts an applicable recovery path and reports that a required source cannot be accessed. The other researchers continue. The parent receives the failed outcome and can choose an authorized alternative source or disclose the missing evidence in its final result.

Ownership: the task module records the child's outcome, dependency relationship, partial-result references, and whether execution has settled. Scheduling makes an eligible parent continuation available to inspect the failure. The parent agent chooses the response within its instructions, permissions, and configured limits. Failure handling is ordinary agent execution; it does not require another LLM coordinator.

A failed dependency must not remain pending forever or appear successful merely because the child produced some output. Preserve completed, partial, and uncertain results with their provenance. A parent must assess whether required evidence or deliverables are still missing before claiming success. Continuing with a limitation does not authorize declaring an unmet requirement fulfilled.

Outcome delivery and parent continuation must survive restart and tolerate duplicate notifications. Parent processing remains serialized within its lane. The scheduler may combine pending child outcomes into an eligible continuation, but must preserve each outcome and avoid starting concurrent parent turns. An active parent receives the outcome through the defined runtime boundary; a waiting parent can become eligible before all healthy siblings finish.

Recovery retains the same goal ancestry and configured aggregate budgets under [Shared goal budgets with optional cost limits](agents.md#shared-goal-budgets-with-optional-cost-limits). Record replacement attempts and their relationship to the failed work rather than erasing failure history. Retries and reassignments must be bounded against repetitive, unproductive recovery. If recovery cannot proceed, expose the reason and ask for help or report the limitation.

Provider waiting under [Retry and wait for the selected model](agents.md#retry-and-wait-for-the-selected-model), budget pauses under [Preserve progress and pause on configured budget exhaustion](agents.md#preserve-progress-and-pause-on-configured-budget-exhaustion), explicit cancellation under [Cancel selected work and its descendants](#cancel-selected-work-and-its-descendants), and uncertain external outcomes remain distinct from settled task failure. A provider outage does not authorize automatic model or connection fallback. A network timeout alone does not prove an external action failed; resolve its outcome before retrying or assigning a duplicate effect to another worker.

Explicit cancellation takes precedence over recovery. A cancelled root cannot resume because a child reports failure. A parent may acknowledge a user-cancelled child and explain the impact, but must not automatically recreate the cancelled work through another agent or task. This decision does not grant authority to undo a user's stop request.

Failure containment does not remove dependencies: work that requires a missing result cannot consume an invented success. Healthy siblings continue unless another applicable condition stops them. The settlement of a failed assignment's own active descendants and tool operations must be explicit before resource release or task-bound agent retirement. This decision does not select a blanket cascade policy for those descendants.

**Rationale.** Automatically stopping the entire goal makes one failed branch interrupt otherwise useful progress. Selecting a stop-or-recover policy on every delegation adds configuration to ordinary collaboration. The user chose parent-directed adaptation as the default behavior.

**Open details.** Outcome schema and failure classification; parent wake-up, delivery and batching mechanics; recovery attempt and no-progress limits; failed-parent descendant settlement; human escalation; and final goal completion criteria.

**Validation scenarios.** One child fails while siblings progress; a waiting parent receives failure without waiting indefinitely; partial results remain identifiable; duplicate delivery does not duplicate recovery; cancellation blocks replacement work; provider waiting does not become model fallback; uncertain effects prevent blind retries; and recovered work retains goal ancestry and accounting. Implementation remains pending.

## Agent completion with optional human acceptance

Accepted 2026-10-24.

**Decision.** The responsible agent may complete a goal by default after checking its requirements and recording results and supporting evidence. A goal can explicitly require acceptance by an authorized human; in that case, the agent submits its result for review and the goal waits for acceptance. The user selected this model on 2026-10-24.

Usage: a research agent assembles its delegates' findings, verifies the requested coverage, publishes a report, and completes an ordinary goal. For a goal requiring human acceptance, the same result appears as awaiting review. The reviewer accepts that submitted result or requests changes. Submission, agent-declared completion, and human acceptance remain distinguishable in the history and console.

Ownership: the responsible agent assesses semantic satisfaction and provides the completion report. The task module validates lifecycle transitions, required result references, current ownership, dependency settlement, configured checks, and acceptance policy. Authorization decides who may change that policy and accept results. The frontend presents evidence and outstanding requirements; it does not infer completion from a final chat message.

Completion reports must account for the goal's requirements, deliverables, performed checks, and unresolved limitations. A missing requirement is not satisfied merely because an agent claims success. Configured machine-verifiable checks must pass where applicable. Human judgment and agent judgment remain labeled as such; this decision does not claim an automatic verifier can prove every organizational goal correct.

Submission for review releases active execution capacity. It does not require the agent's sandbox to stay awake. Review and notifications operate on durable records and artifacts. Results must remain inspectable under the reviewer's permissions; linking a result does not grant access to otherwise restricted data.

Acceptance must refer to a specific submitted result revision. New work, changed requirements, or revised deliverables must not inherit acceptance of an older submission. Proposed mechanics include immutable submissions, revision-checked review actions, and a changes-requested continuation. Detailed reviewer selection and transition types belong in the consolidated specification.

The responsible agent cannot remove a human-acceptance requirement merely to finish. Timed-out or absent review is not implicit acceptance. Explicit cancellation, budget pauses, and unresolved external effects retain their existing meaning. A late model answer or child result cannot change a cancelled goal into a completed one. Awaiting human review is distinct from execution failure or a budget pause.

The policy applies to goal outcomes. Human acceptance of every delegated child assignment is not implied; the parent remains responsible for integrating and evaluating delegated results under [Explicit agent messaging and delegation](#explicit-agent-messaging-and-delegation) and [Contain delegated failures and let the parent adapt](#contain-delegated-failures-and-let-the-parent-adapt). Remaining goal-owned execution must be settled before final completion, rather than continuing hidden work after the goal is reported complete.

**Rationale.** Alternatives considered: require human acceptance for all goals; provide agent-only completion without an acceptance workflow. The user selected autonomous completion as the default with an explicit review option for goals that need it.

**Open details.** Requirement and evidence schema; permitted acceptance-policy changes; reviewer assignment; configured checks; submission and revision behavior; changes-requested resumption; and final outcome labels. These will be proposed in the consolidated specification rather than extending the product-question sequence.

**Validation scenarios.** Complete an ordinary goal with evidence; submit a review-required result without marking it accepted; review while the agent sleeps; request changes; reject acceptance of a stale revision; prevent policy downgrade by the agent; account for failed dependencies and missing requirements; and preserve cancellation during a completion race. Implementation remains pending.

## Tasks and the task orchestrator

Accepted 2026-11-04, as the user specified it that day.

**Decision.** A task is something a person wants done, regardless of how or by whom. They say what they want on the Tasks page; the task orchestrator names the task, picks one or more agents and gives each a brief and items. Work and conversation happen in a room that exists only for that task and is reachable only from the Tasks page: it is on no room list, cannot be favorited or have its members edited, and its agents do not count as sharing a room for `delegate` or `send_message`.

The task orchestrator is a platform agent in the sense of [Direct mentions and a tool-free room coordinator](#direct-mentions-and-a-tool-free-room-coordinator): one of the orchestrator's model duties, with its own prompt and optionally its own model, no sandbox, tools or memory. It acts when a person writes without mentioning anyone, or when every turn of the task has settled. It only proposes, as JSON naming agents by id; Rust checks that each is one the task's owner may use, applies the configured limits on agents per round and rounds of work, and refuses to finish a task with open work. Past the round limit, it asks the person. After three invalid answers it hands a task nobody has started to the first eligible agent, as a room does, and otherwise tells the person it could not decide.

A task agent's turn is a fork: its context starts from its own conversation as it stands, with the room's recent history, and nothing returns. Task turns are room deliveries, so they stay out of the conversation, its unread count and its summaries ([Direct mentions and a tool-free room coordinator](#direct-mentions-and-a-tool-free-room-coordinator), amended 2026-11-04). Memory stays one pool per agent, so what it learns remains available; a fork of memory was not asked for. The turn still runs in the agent's main lane, so it queues behind what the agent is doing.

In a task turn `delegate` and `send_message` are replaced by `task_message`, which posts in the room and starts a turn for each agent it names, because the other tools would put the work in the recipient's own conversation; and `task_item`, which marks the agent's own items. Agents can start at most 64 turns for each other in one task, which bounds a pair that keeps answering.

A task belongs to the person who asked and is visible to administrators. The orchestrator may only choose agents that person can send to, and the work carries their authority. Writing in a finished or stopped task starts it again with fresh rounds. Stopping cancels its open turns; it does not undo what they did.

**Rationale.** People should not have to know which agent to ask or how to split work. Building on rooms gives tasks the durable messages, deliveries, retries, cancellation and streaming that rooms already have, instead of a parallel system.

**Open details.** Human acceptance of a task's result ([Agent completion with optional human acceptance](#agent-completion-with-optional-human-acceptance)) is not offered; the orchestrator's judgment closes a task. Sharing a task with other people; running a task turn beside the agent's main lane; a forked workspace or desktop, which agents share with their other work.

**Validation scenarios.** Automated with the real agent binary and a scripted model: a task is planned, its agent works from its conversation, marks items and speaks in the room, and its next ordinary turn contains none of it; unknown agents, round limits, open work and garbled answers are overruled; a person's other tasks and rooms stay hidden from others. Not yet validated: a real model acting as orchestrator, and several agents working one task at once on real sandboxes.
