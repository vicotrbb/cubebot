# ADR 0030: Retry and wait for the selected model

Status: Accepted

Date: 2026-10-23

ADR 0029 defines each agent's default model and optional purpose overrides. Provider outages, rate limits, and connection failures can prevent the selected model from responding. Recovery must preserve deliberate model selection and already-completed work.

Decision: Retry eligible transient failures within limits, then leave affected work visibly blocked pending recovery or an authorized configuration change. The user selected this behavior on 2026-10-23. Cubebot does not automatically switch models, providers, or connections, and does not offer automatic fallback chains under this decision.

The selected connection and model remain in effect for the operation. An unavailable explicit purpose override does not fall back to the agent's default. A manager may deliberately change configuration, but activation and safe continuation rules must be defined separately; a configuration edit must not silently restart completed work.

Usage: A working agent encounters a temporary provider failure. The runtime retries the model request within its retry and execution budgets. If access remains unavailable, the console shows the affected work as waiting for that model, with its reason and last attempt. Recorded task progress and completed tool actions remain intact. When recovery permits another attempt, execution resumes from recorded state rather than replaying the whole task.

Ownership: Provider adapters classify failures and expose retry information where available. The execution module owns bounded request retries and partial-response handling. Scheduling owns durable waiting and subsequent admission. Authorization and budgets continue to constrain every attempt; provider recovery does not restore expired authority or reset spending limits.

Implementation requirements: Use bounded backoff for retryable failures and avoid synchronized retries across agents sharing a connection. Authentication failures, revoked grants, exhausted budgets, incompatible capabilities, and invalid requests must not enter an indefinite transient-error retry loop. Their recorded reason identifies the required intervention or recovery condition. Exact classification, timing, and attempt limits remain open.

Waiting is a durable execution state, not an unbounded busy loop. The console distinguishes retrying, waiting on provider recovery, and conditions requiring intervention, without exposing secrets. Users can still inspect and cancel affected work. Detailed lane reservation, pod sleep eligibility, recovery probing, notification, and deadline behavior remain separate scheduling decisions. A waiting lane must not be treated as safely idle solely because no inference call is in flight.

Interrupted responses require explicit reconciliation. A retry may incur provider cost again and does not imply exactly-once inference. The engine must not execute incomplete tool calls or repeat already-executed tools merely because the response stream or a later inference request failed. Uncertain action outcomes retain their recovery state. Disconnected retries remain subject to ADR 0012's finite authority and journaling limits.

This policy also applies to agent maintenance purposes. Failed compaction retains the last valid context under ADR 0024; if ordinary work cannot fit without compaction, it waits visibly. Maintenance failure must not falsely report successful extraction, dreaming changes, or context replacement.

Alternative considered: An explicitly configured fallback chain could preserve progress through some outages but would change model behavior, provider exposure, and possibly cost. The user selected predictable model use with retry and waiting.

Consequences: Model outages or prolonged rate limits can delay autonomous work. No alternate provider is contacted automatically. Recovery scheduling, wait visibility, and preservation of partial progress become essential for unattended operation.

Open details: Retry classification and limits; backoff and jitter; shared-connection recovery coordination; partial-stream reconciliation; wait deadlines and notifications; lane and pod lifecycle while waiting; configuration-change activation; and coordinator or embedding failure policies.

Validation scenarios: A transient failure succeeds on a bounded retry; repeated failures enter visible waiting; an unavailable override never invokes the default; authentication failure avoids a retry storm; cancellation stops further attempts; restart preserves waiting state; recovery does not replay a completed tool; and retry budgets remain bounded during orchestrator disconnection.
