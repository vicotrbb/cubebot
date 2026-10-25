# ADR 0065: Coordinated maintenance windows for backups

Status: Accepted

Date: 2026-10-23

Cubebot coordinates backups across structured records, shared files, and agent workspaces. A consistent recovery point requires controlling writers while those stores are captured.

Decision: Use a coordinated maintenance window for backup capture. The user selected this model on 2026-10-23. Cubebot stops admitting new work, brings affected execution and writers to safe checkpoints, captures consistent state, and resumes after the recovery boundary is secured. Continuous-work backups are outside the initial contract.

Usage: An administrator starts a backup. The console reports preparation, draining, capture, and subsequent verification or transfer. Agents finish an eligible step or checkpoint their work. Cubebot captures the required stores while writers are controlled, then resumes work when doing so cannot change the captured recovery point. Failure or timeout is visible and does not produce a successful backup claim.

Ownership: The recovery module coordinates the durable maintenance workflow. Scheduling enforces admission pauses, runtime and tool modules establish agent checkpoints, and persistence modules control their writers. These modules acknowledge their state; a UI flag alone cannot establish that the installation is safe to capture.

The maintenance barrier covers agent turns, dreaming and extraction, workspace-writing programs, browser activity that affects captured state, uploads and publication, skill revisions, configuration mutations, and destructive cleanup. It also accounts for relevant background workers and agent lifecycle changes. Pausing model inference alone is insufficient when child processes can still modify files. The exact drain, checkpoint, and fencing mechanisms remain design work.

New user work and incoming events must receive explicit maintenance behavior. They may be durably queued if the capture protocol supports it, or rejected with a retryable maintenance result. They must never be acknowledged as accepted and then lost. The protocol must state which side of the recovery boundary accepted arrivals belong to. Detailed ingress behavior remains open; the console should retain read-only progress visibility where available.

Planned maintenance must be reconciled with ADR 0012's bounded disconnected execution. A disconnected agent may still be writing under existing authority. Capture cannot assume it has paused merely because its heartbeat is absent. Obtain a checkpoint or establish that writing has stopped using qualified expiry and fencing behavior; otherwise the full backup cannot complete.

Drain and capture have bounded time budgets. If a task, tool, or workspace cannot reach a safe state, fail or defer the backup with the blocker identified. Do not silently omit that workspace or claim a clean checkpoint after killing a process. Any explicit cancellation must preserve uncertain external-action outcomes. Existing work resumes only when maintenance ownership and execution state are reconciled.

With qualified snapshots or equivalent immutable captures, the pause may end once all required recovery points are secured and protected from cleanup. Export and verification can continue afterward. With ordinary file copies, the pause may need to cover copying. Neither a short pause nor snapshot availability is guaranteed across all storage backends. Backup completion still requires the full capture and verification contract in ADR 0064.

Maintenance state must survive orchestrator restart. Recovery determines whether to finish capture or abort it before releasing admission; agents must not resume merely because a network connection returned. Timeout or cancellation must also release temporary holds safely, without racing a still-active capture or reopening a partially restored installation.

Alternative considered: Capture consistent backups while ordinary work continues. The selected maintenance approach makes the initial cross-store consistency contract easier to qualify while accepting a visible work pause.

Open details: Checkpoint and fencing protocol; ingress behavior; drain and capture deadlines; administrator cancellation; snapshot qualification; export and verification mechanics; and maximum acceptable interruption.

Validation scenarios: Pause admissions and all relevant writers; reject an unconfirmed checkpoint; handle an offline writer; fail on a bounded drain timeout; recover maintenance after restart; resume safely after failure; export secured snapshots after resumption; and restore a verified cross-store recovery point.
