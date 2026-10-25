# ADR 0046: Scheduled and event-triggered work

Status: Accepted

Date: 2026-10-23

Persistent agents need to perform recurring duties and respond to changes outside conversations. Requiring a person to send a chat message for every occurrence would prevent those workflows.

Decision: Support both schedules and incoming external events as triggers for agent work. The user selected both on 2026-10-23. Trigger occurrences create durable work through the orchestrator and use the existing assignment, admission, and execution machinery.

Usage: A scheduled routine asks an agent to review selected repositories every weekday morning. An authenticated monitoring event asks an agent to investigate an alert. Either can request a wake for a sleeping agent, subject to permissions, budgets, capacity, and lifecycle policy. Triggering work does not guarantee immediate execution.

Ownership: The orchestrator owns trigger configuration, schedule evaluation, event admission, and durable occurrence records. The task module owns resulting assignments; the scheduler admits them to agent execution lanes. Event adapters translate supported external inputs into this common path. Agents do not need an always-running pod solely to receive events or track schedules. This decision does not introduce a separate workflow engine, message broker, or Kubernetes CronJob for each routine.

A trigger identifies its intended work and destination. Each accepted occurrence must retain enough origin and correlation information to explain why work started and relate it to its assignment and outcome. The console must expose configured triggers and occurrence status, including queued work, failures, and rejected or skipped occurrences where recorded. Exact routing to agents or rooms remains to be specified consistently with existing room-routing rules.

Incoming events are external data. Admission must validate the configured source and payload before creating work. Event content cannot grant tools, select unauthorized accounts, or expand an agent's permissions. Delivery through an approved integration does not make instructions inside the payload trusted system instructions. MCP tool support alone does not imply inbound event support; each supported event source needs an explicit ingress path.

Schedule occurrences and repeated event deliveries need stable identities and deduplication appropriate to their sources. Recovery must account for an occurrence already admitted before a restart. This is not a guarantee of exactly-once external side effects. [ADR 0048](0048-per-trigger-occurrence-handling.md) selects individual assignments or combined pending work per trigger. Missed schedules, overflow, and sources without reliable event identities still require explicit policies before implementation.

[ADR 0047](0047-autonomous-trigger-management.md) includes managing triggers for an agent's own work in ordinary autonomy, within existing scope and installation limits. This does not authorize registering integrations or exposing arbitrary endpoints; external subscription changes require applicable integration permissions. Retired agents cannot be revived by triggers; unavailable targets and admission failures need visible outcomes.

Alternative considered: Introduce schedules or external events first and defer the other. Both are selected because recurring routines and reactive work are part of the intended product. They share durable work admission while retaining source-specific validation.

Open details: Trigger edit scope; schedule syntax and time zones; missed-occurrence policies and grouping mechanics; event authentication, replay protection, and payload limits; supported sources; trigger destinations; rate limits and backpressure; occurrence retention; and cancellation semantics.

Validation scenarios: Admit scheduled and event-triggered work; wake a sleeping agent; preserve accepted work across orchestrator restart; deduplicate a repeated identifiable occurrence; reject an unauthenticated event; bound a burst; report an unavailable target; and prevent event content from expanding authority.
