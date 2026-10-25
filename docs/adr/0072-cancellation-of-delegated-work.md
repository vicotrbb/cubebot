# ADR 0072: Cancel selected work and its descendants

Status: Accepted

Date: 2026-10-23

Decision: Stopping a goal cancels that goal and all work delegated under it. Stopping an individual child assignment cancels that child and its descendants. Preserve partial results and leave unrelated work running. The user selected this behavior on 2026-10-23.

Usage: A lead delegates a market analysis to three researchers, one of whom delegates a further task. An authorized user stops the root goal. Cubebot blocks further work for that goal, propagates cancellation to every unsettled descendant, and shows stopping until affected execution has settled. Other goals using the same agents remain eligible to run.

Scope follows durable task ancestry, including queued work, waiting continuations, active assignments, and work transferred to another agent. Completed results remain completed evidence; cancelling their parent does not rewrite them as failed or remove their outputs. Informational messages do not create descendants. Goal-attributed requests must retain their ancestry so another collaboration path cannot escape cancellation.

Ownership: The task module authorizes and durably records cancellation scope. Scheduling prevents affected starts and continuations. Runtime and tool modules stop further affected actions and settle or reconcile in-flight activity. The orchestrator coordinates propagation and recovery. The console distinguishes accepted cancellation, stopping, and settled cancellation; notifications follow existing access and preference rules.

Cancellation and delegation must have a defined ordering. A delegation committed before cancellation is included in its scope. A request to create further work under a cancelled ancestor is rejected. Transfers, retries, late results, steering, budget increases, and orchestrator restarts cannot clear cancellation or recreate the work implicitly. Admission must check effective ancestor cancellation rather than depend solely on asynchronous delivery to every descendant.

Cancellation is scoped to work, not to every activity in an agent pod. Do not stop an entire agent's unrelated lanes merely because one assignment is cancelled. Persistent agents remain available. Task-bound agents follow ADR 0018's retirement rules once originating and dependent work are settled, preserving history and recovery evidence.

Already-issued requests may finish or have an uncertain outcome. Retain their outputs and effect records without treating late arrival as permission to resume the cancelled goal. Cancellation cannot undo external effects. Tool-specific interruption, process cleanup, and recovery must respect unrelated activity and preserve evidence needed to resolve uncertainty.

Disconnected agents remain subject to the finite authority and journaling rules in ADR 0012. The orchestrator cannot claim that recording cancellation instantly stopped them. Track outstanding execution until acknowledgement, expiry or fencing, and reconciliation establish its state. Revoking permission for future work does not prove an external request already issued was rolled back.

Cancelling a child makes its cancelled outcome available to the waiting parent without presenting success or leaving a dependency waiting forever. The parent is outside that child's cancellation scope. [ADR 0073](0073-contained-delegation-failures.md) allows adaptation to failed dependencies while prohibiting automatic recreation of explicitly cancelled work. Cancelling the root prevents all descendant result arrivals from scheduling further root-goal work.

This decision defines cancellation, distinct from the resumable budget pause in ADR 0068. Recovery retains partial progress, but resuming cancelled work requires a separate explicit continuation contract; no automatic restart is permitted. Data deletion and retention are separate policies.

Alternative considered: Stop only the selected assignment while delegates continue. This preserves independently useful delegate activity but conflicts with the user's selected expectation that stopping a goal also stops acting on its behalf.

Open details: Cancellation permission mapping; concrete state transitions and aggregation; tool interruption deadlines; disconnected executor fencing; dependency failure handling; explicit continuation of cancelled work; and presentation of unresolved external outcomes.

Validation scenarios: Cancel a root with nested delegates; cancel a child while siblings continue; race cancellation with delegation or transfer; cancel queued and waiting work; reconcile late results without resuming the goal; repeat cancellation after restart; retain uncertain external effects; handle disconnected workers; and preserve unrelated lanes and partial outputs. Implementation and runtime qualification remain pending.
