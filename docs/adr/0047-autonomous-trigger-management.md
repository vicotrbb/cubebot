# ADR 0047: Trigger management within ordinary agent autonomy

Status: Accepted

Date: 2026-10-23

Agents performing continuing work need to schedule follow-ups and react to relevant events. Requiring a separate trigger-management grant would make this capability opt-in even for otherwise autonomous agents.

Decision: Include management of triggers for an agent's own work in ordinary agent autonomy. The user selected this model on 2026-10-23. Agents may create and manage their schedules and event triggers by default within installation limits and existing authority, without a separate capability grant or per-change human approval.

Usage: While investigating an issue, an agent schedules a later check or arranges recurring reviews. It can also configure a trigger using an already approved event source it is authorized to use. The orchestrator validates and persists the trigger, then admits future occurrences through ADR 0046's durable work path.

Ownership: The orchestrator owns trigger records and scheduling. Authorization enforces applicable scope and installation restrictions. The agent chooses the trigger's purpose and configuration through native tools. Administrators and authorized managers can inspect and manage these triggers through their existing scoped authority.

This default covers an agent's own work. It does not confer authority to modify another agent's triggers, alter a human-managed trigger outside its scope, or schedule work in an unauthorized room. Cross-agent delegation continues to use its existing authorization rules. Trigger records must identify their creator, target, configuration revisions, and current status so human intervention is understandable.

Installation limits apply to active triggers, occurrence rates, queued work, and resulting execution costs. Exact limits remain to be specified. Creating several triggers cannot bypass aggregate limits. A trigger stores work intent, not a permanent snapshot of permissions: future occurrences and their execution remain subject to current authority and lifecycle rules. Retired agents do not wake because an old trigger remains configured.

Event triggers use administrator-configured sources and installation-managed connections under ADRs 0042 and 0043. Ordinary trigger autonomy does not authorize registration of a new integration, credential acquisition, or arbitrary endpoint exposure. A Cubebot event filter and an external service subscription are distinct operations; changing the latter requires the applicable existing integration permissions.

Management operations must reach the orchestrator to become durable. An agent disconnected under ADR 0012 cannot independently activate a new central schedule or event subscription. Retrying a creation request after an uncertain response must not create duplicate triggers.

Human cancellation or disabling must be respected. An agent must not silently undo an explicit human stop through routine reconciliation. The exact treatment of pending occurrences and already-running work when a trigger is paused or removed remains an explicit lifecycle design detail.

Alternatives considered: Human-only management, or agent management enabled by a separate explicit grant. Both restrict autonomous follow-up more than the selected default. Existing scope checks, visible records, and installation limits remain necessary with ordinary autonomy.

[ADR 0048](0048-per-trigger-occurrence-handling.md) lets each trigger select individual assignments or combined pending work. Autonomous trigger management can select this policy within installation limits.

Open details: Default quotas; trigger ownership and edit scope; management interface; pause and cancellation semantics; handling pending work; frequency limits; and backlog overflow policies.

Validation scenarios: Create a self-directed trigger without an extra grant; update or remove it within scope; enforce aggregate quotas; deny unauthorized targets and integration registration; respect a human stop; recheck authority for later work; and deduplicate creation after a lost response.
