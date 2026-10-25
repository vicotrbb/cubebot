# ADR 0080: Agent completion with optional human acceptance

Status: Accepted

Date: 2026-10-24

Decision: The responsible agent may complete a goal by default after checking its requirements and recording results and supporting evidence. A goal can explicitly require acceptance by an authorized human; in that case, the agent submits its result for review and the goal waits for acceptance. The user selected this model on 2026-10-24.

Usage: A research agent assembles its delegates' findings, verifies the requested coverage, publishes a report, and completes an ordinary goal. For a goal requiring human acceptance, the same result appears as awaiting review. The reviewer accepts that submitted result or requests changes. Submission, agent-declared completion, and human acceptance remain distinguishable in the history and console.

Ownership: The responsible agent assesses semantic satisfaction and provides the completion report. The task module validates lifecycle transitions, required result references, current ownership, dependency settlement, configured checks, and acceptance policy. Authorization decides who may change that policy and accept results. The frontend presents evidence and outstanding requirements; it does not infer completion from a final chat message.

Completion reports must account for the goal's requirements, deliverables, performed checks, and unresolved limitations. A missing requirement is not satisfied merely because an agent claims success. Configured machine-verifiable checks must pass where applicable. Human judgment and agent judgment remain labeled as such; this decision does not claim an automatic verifier can prove every organizational goal correct.

Submission for review releases active execution capacity. It does not require an agent pod to stay awake. Review and notifications operate on durable records and artifacts. Results must remain inspectable under the reviewer's permissions; linking a result does not grant access to otherwise restricted data.

Acceptance must refer to a specific submitted result revision. New work, changed requirements, or revised deliverables must not inherit acceptance of an older submission. Proposed mechanics include immutable submissions, revision-checked review actions, and a changes-requested continuation. Detailed reviewer selection and transition types belong in the consolidated specification.

The responsible agent cannot remove a human-acceptance requirement merely to finish. Timed-out or absent review is not implicit acceptance. Explicit cancellation, budget pauses, and unresolved external effects retain their existing meaning. A late model answer or child result cannot change a cancelled goal into a completed one. Awaiting human review is distinct from execution failure or a budget pause.

The policy applies to goal outcomes. Human acceptance of every delegated child assignment is not implied; the parent remains responsible for integrating and evaluating delegated results under ADRs 0016 and 0073. Remaining goal-owned execution must be settled before final completion, rather than continuing hidden work after the goal is reported complete.

Alternatives considered: Require human acceptance for all goals; provide agent-only completion without an acceptance workflow. The user selected autonomous completion as the default with an explicit review option for goals that need it.

Open mechanics: Requirement and evidence schema; permitted acceptance-policy changes; reviewer assignment; configured checks; submission and revision behavior; changes-requested resumption; and final outcome labels. These will be proposed in the consolidated specification rather than extending the product-question sequence.

Validation scenarios: Complete an ordinary goal with evidence; submit a review-required result without marking it accepted; review while the agent sleeps; request changes; reject acceptance of a stale revision; prevent policy downgrade by the agent; account for failed dependencies and missing requirements; and preserve cancellation during a completion race. Implementation remains pending.
