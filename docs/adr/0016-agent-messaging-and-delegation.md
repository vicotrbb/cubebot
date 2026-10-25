# ADR 0016: Explicit agent messaging and delegation

Status: Accepted

Date: 2026-10-23

Human room requests follow mention and coordinator routing under [ADR 0014](0014-room-routing.md) and [ADR 0015](0015-multiple-room-mentions.md). Agent-authored progress, requests, and results need distinct execution semantics.

Decision: Adopt OpenBot-style explicit agent messaging and delegation. The user accepted this model on 2026-10-23. Agents initiate collaboration through structured messaging, assignment, or transfer operations. Ordinary progress and final-answer text is recorded in the room without automatically invoking the coordinator or assigning work to mentioned agents. Expected results can resume waiting work through existing task and message relationships.

Room assignment creates a child task with a named recipient, instructions, expected result, and relevant source references. The assigning owner remains responsible for integrating the result. Transfer changes ownership of the whole task and ends the old owner's execution of it. Publishing a result records output for an existing assignment; it is not a new room request.

An owner waiting for delegated work ends its current turn and releases execution capacity. The durable task relationship determines when a continuation becomes eligible. It does not require a model to read the result and rediscover who is waiting. Task completion and result publication must remain distinct enough to handle failures after output was published.

Direct agent messaging outside room assignments uses explicit recipients, a message identity, reply correlation, and whether an answer is expected. Results and informational messages do not request an acknowledgement by default. A correlated result can schedule the requester's continuation without creating a new request to its sender. An explicit new request may continue the conversation under the applicable permissions and budgets.

Following the selected reference behavior, ordinary direct messaging is not a bypass for room delegation: work within a room assignment uses room assignment or transfer operations. A successful direct-request turn can relay its final result if no explicit answer was already sent. Failed or interrupted turns must not be presented as successful results. Automatic relay must be idempotent and must not echo informational or result messages back indefinitely.

Usage: A researcher explicitly assigns implementation to an engineer and yields. The engineer publishes a result and completes its task. Once the required dependencies are satisfied, the researcher resumes to integrate the result and answer the user. A progress sentence naming the engineer creates no additional assignment.

Ownership: The messaging module owns delivery identity and reply correlation. The task module owns assignment, dependencies, transfer, and continuation eligibility. The scheduler owns admission to execution lanes. The transcript stores visible output; displaying text does not itself grant authority to schedule work.

Alternative considered: Routing all agent output like human requests would permit conversational mentions to trigger work but increase routing calls and accidental response chains. The selected model makes actionable intent explicit while preserving autonomous collaboration.

Consequences: Agents need tools for collaboration, not just instructions to mention names. Permission checks, duplicate suppression, and bounded execution still apply. Passive output is available as future context even when it starts no new turn. Declining automatic acknowledgements alone is not a complete loop-prevention policy.

Related decision: [ADR 0017](0017-agent-initiated-creation.md) permits agent creation as a separately granted capability. Permission to collaborate does not itself grant permission to create agents.

[ADR 0067](0067-shared-goal-budgets-with-optional-cost-limits.md) shares configured goal limits across delegated work and continuations. Monetary limits are optional and unset by default; delegation does not create additional allowance.

[ADR 0072](0072-cancellation-of-delegated-work.md) cancels selected work and its descendants, blocks further affected delegation and continuation, and preserves partial results and unrelated work.

[ADR 0073](0073-contained-delegation-failures.md) returns failed child outcomes and partial results to the parent for adaptation while healthy siblings continue.

Open details: Exact tool interfaces; outcome delivery and partial-result schema; cancellation settlement and dependency rules; delegation depth and budget accounting; direct-message result batching; timeout behavior; retries; and permitted recipient scope.

Validation scenarios: Passive progress containing a mention starts no work; explicit assignment starts only its recipient; transfer leaves one current owner; required results resume a waiting owner; repeated result delivery does not duplicate continuation; fallback direct replies do not duplicate explicit replies; and informational messages produce no automatic reply echo.
