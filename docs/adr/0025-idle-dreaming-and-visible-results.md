# ADR 0025: Idle-only dreaming with visible status and results

Status: Accepted

Date: 2026-10-23

Dreaming consolidates an agent's derived memory and can create or improve reusable skills under ADR 0055. It shares resource budgets with ordinary work, and users need to understand when it runs and what it changes.

Decision: Dream only while the agent is idle across all execution lanes. The user selected idle-only dreaming on 2026-10-23 and explicitly required visibility into both the dreaming state and its results through the management console and mobile PWA.

Scheduling owns admission. Dreaming must not start alongside an active main or task turn, or take precedence over eligible queued work. New work requests cause dreaming to yield at a safe checkpoint before ordinary execution begins. A quiet main lane alone does not establish idle eligibility. Unresolved execution outcomes must be reconciled before treating a lane as idle.

Scheduled dreaming can wake a sleeping agent under [ADR 0007](0007-agent-idle-lifecycle.md), subject to admission and budgets. Idle status does not trigger continuous inference. Retired agents remain ineligible under ADR 0018. Automatic extraction under ADR 0021 and necessary active-context compaction under ADR 0024 remain separate operations; this decision governs broader dreaming maintenance.

Usage: A user sees an agent marked as dreaming, opens its current maintenance run, and sees the recorded activity. When the run finishes, the user can inspect a concise result summary and the actual memory changes. If a message arrives first, the run yields and the agent handles the message. Its report distinguishes changes already committed from unfinished work.

Visibility contract:

- Agent lists and agent details distinguish dreaming from sleeping, ordinary work, and unavailability. A run waiting for admission is not shown as actively dreaming.
- Each run has a durable identity, timestamps, status, and recorded progress. Completion, yielding, and failure remain distinguishable; stale or disconnected status must not masquerade as confirmed live progress.
- Results show a readable summary backed by committed changes: memories added, revised, merged, or superseded, with before/after content where applicable and references to supporting sources. An unchanged run explicitly reports no changes. Unresolved contradictions can remain unresolved in the report.
- Failed or yielded runs expose any committed partial results separately from pending proposals. A model's narrative alone is not proof that a change was applied.
- Under [ADR 0055](0055-reusable-skills-and-autonomous-skill-creation.md), results also show skills created or revised, supporting sources, revision changes, installation outcomes, and any validation actually performed. Saved revisions and active installations remain distinguishable.
- Past results remain inspectable when the agent sleeps. Result retention and pagination remain to be specified.

Reports expose outcomes and evidence, not hidden model reasoning. They follow existing content-access rules: administrators have full access under ADR 0003, while ordinary users need the relevant grants. Status visibility must not accidentally disclose protected memory content through previews or summaries. Exact ordinary-user permissions remain open.

Ownership: Scheduling owns idle eligibility and yielding. The memory module owns memory changes, provenance, and conflict checks. The skills module owns skill revisions and installation outcomes under ADR 0055. Run records associate committed changes with the originating dreaming run. The console reads these records through the orchestrator. Detailed event transport, checkpoint format, and execution placement remain open.

Dreaming preserves source history under ADR 0020 and does not inject memories into working contexts under ADR 0023. On yielding or failure, incomplete changes must not appear committed. Retrying a run must not duplicate already committed changes. Idle-only execution does not remove the need to handle concurrent automatic extraction or other authorized memory updates.

Alternatives considered: Concurrent dreaming could keep maintenance progressing for continuously busy agents, but adds competition with active work. Per-agent configurability would require supporting both modes. The user selected idle-only operation. A status badge alone would not satisfy the additional requirement to inspect results.

Consequences: Busy agents can accumulate a maintenance backlog. The console should expose delayed maintenance. Safe yielding can add some response latency, so checkpoint duration needs a bound. Reports and change records require storage and must remain consistent with actual commits.

Related decisions: [ADR 0026](0026-human-memory-editing.md) supports direct memory editing and conversational correction with an inspectable revision trail. [ADR 0029](0029-model-default-and-purpose-overrides.md) uses the agent's default model for dreaming unless a purpose override is configured.

Open details: Trigger cadence; idle grace period; budgets and fairness; yield latency; maintenance backlog indicators; run and change-record schemas; ordinary-user access; report retention; live-update transport; and rollback support.

Validation scenarios: An active task prevents dreaming; eligible work wins an admission race; a message causes safe yielding; committed partial results survive a crash; retries do not duplicate changes; no-op and failure outcomes are clear; reports match committed changes; authorized users inspect past results while the pod sleeps; unauthorized users cannot read protected results; and dreaming preserves source history.
