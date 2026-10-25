# ADR 0064: Cubebot-coordinated backup and restore workflows

Status: Accepted

Date: 2026-10-23

Cubebot's recoverable state spans PostgreSQL, orchestrator-managed file storage, and persistent agent workspaces. Independent copies of those stores do not by themselves establish a consistent application recovery point.

Decision: Cubebot provides coordinated backup and restore workflows using established database and storage mechanisms. The user selected this model on 2026-10-23. Cubebot owns application-level coordination, completeness reporting, and restore validation; operators configure backup destinations, scheduling, retention, and required infrastructure access. Those configuration details remain open.

Usage: An authorized administrator starts a backup or configures a backup policy. Cubebot coordinates capture, records the included resources and recovery boundary, and reports a completed or failed backup with inspectable evidence. During restore, Cubebot validates the selected backup, restores its required components, and checks their relationships before allowing ordinary work to resume.

Ownership: A recovery module owns workflow identity, manifest state, progress, and reconciliation inside the existing orchestrator design. Database, shared-storage, and workspace modules expose the capture and restore operations needed for their state. Established database tools, supported snapshots, or file-copy mechanisms perform the underlying data capture. This decision does not introduce a custom PostgreSQL backup engine or require a new always-running service.

A complete backup must account for the application schema and version, database capture, shared-file objects and references, workspace inventory, configuration, and required secret or key recovery dependencies. A manifest records what was captured, how consistency was established, integrity evidence, and required restore prerequisites. Sensitive recovery material must not appear in ordinary logs or unprotected manifests. [ADR 0066](0066-encrypted-credentials-with-separate-installation-key.md) requires separate recovery of the installation key versions needed by encrypted credential records. Exact secret packaging and key custody remain open.

Backup status must distinguish capture in progress, incomplete or failed capture, complete capture with verification, and a separately exercised restore. A successful database dump alone cannot mark the installation backup complete. Missing workspaces, unreadable files, failed checks, or unavailable required keys must be reported rather than omitted silently. Backup and restoration retries must reconcile their existing workflow identity instead of repeating destructive steps blindly.

The consistency protocol must cover active tasks, arbitrary workspace-writing tools, uploads, skill revisions, and physical file cleanup. Bundled and external PostgreSQL need qualified capture paths and sufficient privileges. An external provider's database snapshot does not automatically coordinate Cubebot's volumes. [ADR 0065](0065-maintenance-window-backups.md) selects a coordinated maintenance window for establishing and capturing consistent state.

Restore must be operable when the ordinary installation cannot start. A recovery entrypoint must therefore avoid depending on a healthy production database or web console; its exact CLI or Kubernetes job form remains open. Restoring into an existing installation requires explicit operator intent and must prevent competing orchestrators or agents from mutating the target state.

Restored tasks, triggers, and agent lifecycles must be reconciled before execution resumes. A backup cannot undo external actions performed after its recovery point. Old assignments must not be replayed merely because their later completion is absent from restored state. External account validity and uncertain outcomes may require intervention. Restored access records do not prove that an external credential remains valid.

Backup destinations must be evaluated against the intended failure scenario. A copy on the same lost volume cannot recover that volume; protection against cluster loss requires surviving recovery data and keys elsewhere. No destination, external storage dependency, recovery-time target, or data-loss interval is selected here.

Alternative considered: Delegate the entire workflow to operators. Cubebot-owned coordination better supports personal installations and centralizes the application consistency rules while retaining established capture mechanisms.

Open details: Maintenance mechanics; destinations; scheduling and retention; key custody; snapshot and database tooling; workspace checkpointing; restore entrypoint; resumed-work reconciliation; recovery objectives; and restore-drill procedure.

Validation scenarios: Capture all required stores; report a missing component as incomplete; recover an interrupted backup; restore with the console unavailable; verify matching metadata and bytes; handle missing keys; prevent conflicting writers; and avoid automatic replay of uncertain external effects after rollback.
