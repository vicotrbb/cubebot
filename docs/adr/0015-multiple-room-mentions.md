# ADR 0015: Independent assignments for multiple room mentions

Status: Accepted

Date: 2026-10-23

[ADR 0014](0014-room-routing.md) routes explicit mentions without an LLM routing call and uses a tool-free coordinator for unaddressed requests. A message may explicitly address more than one agent.

Decision: Each mentioned agent receives its own assignment. The user selected this model on 2026-10-23. Multiple mentions do not invoke the coordinator to choose a lead. Each assignment has one owner and can produce its own response; there is no automatically assigned owner of a combined answer.

Usage: A room request addressed to a researcher and an engineer creates an assignment for each, linked to the same originating message. Both receive the request and relevant room context. Each proceeds when its lane and execution capacity permit. One may finish while the other is still queued or working.

The system preserves the relationship between the originating request, recipient, assignment, and response. Repeated mentions of the same agent in one request do not create duplicate assignments. Retrying delivery of the same accepted request must not duplicate recipient assignments. Permission and membership checks still apply to each intended recipient.

Rationale: Explicit addressing remains predictable and requires no routing inference. A user requesting several independent contributions gets those contributions directly.

Alternative considered: Invoking the coordinator to select a lead among the mentioned agents would establish one integrated result but introduce routing inference and change the meaning of explicitly addressing several agents. The user chose independent assignments.

Consequences: Responses may overlap or disagree. No implicit dependency, aggregation barrier, or synthesized final answer is created. Users can request coordinated work by addressing one agent and asking it to delegate, or by sending an unaddressed request for coordinator routing. An assignment owner can still delegate its own work under the subsequent delegation contract.

Parallel eligibility does not guarantee simultaneous execution. Agent lane serialization and installation capacity from [ADR 0005](0005-agent-execution-lanes.md) continue to apply. Cancelling or completing one assignment must not silently settle another; request-wide cancellation semantics remain open.

Related decision: [ADR 0016](0016-agent-messaging-and-delegation.md) requires explicit collaboration operations for agent-initiated work. Mentions in ordinary agent output do not invoke this assignment behavior.

Open details: Structured mention representation; quoted or incidental names; recipient limits; mixed valid and invalid recipients; and request-wide cancellation.

Validation scenarios: Two distinct mentions create two separately owned assignments; repeated mention creates one assignment for that recipient; a request retry creates no duplicates; one busy agent does not require another to wait for its response; and independent results remain attributable to the correct assignment.
