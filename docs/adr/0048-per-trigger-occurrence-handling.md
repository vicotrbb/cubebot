# ADR 0048: Configurable occurrence handling per trigger

Status: Accepted

Date: 2026-10-23

A trigger can produce occurrences faster than an agent can process them. Some workflows require separate work for each occurrence; others benefit from one follow-up that examines the latest state and accumulated signals.

Decision: Each trigger selects individual processing or combined pending work. The user selected this configurability on 2026-10-23. Both modes remain subject to installation limits, existing execution admission, and current permissions.

Individual processing creates a separate assignment for each accepted distinct occurrence. For example, each incoming support request requires its own work. Queue limits still apply; this mode does not promise unlimited acceptance or immediate execution.

Combined processing accumulates occurrences into one pending assignment for that trigger. If an assignment is already running, new occurrences contribute to one pending follow-up rather than repeatedly creating assignments. For example, repeated service-health signals can produce one follow-up investigation with references to the contributing events. Combining does not cancel or rewrite already-running work.

Ownership: The orchestrator's trigger module owns occurrence identity, grouping, and the durable relationship between occurrences and assignments. The task module owns assignment lifecycle, and the scheduler retains responsibility for lane admission. This policy does not change the agent's concurrency limits or create another execution lane.

When a combined assignment starts, its contributing occurrence set needs a clear boundary. Later arrivals belong to subsequent pending work. Admission and grouping must handle arrivals racing with assignment startup or completion without losing occurrences or creating competing pending groups. The implementation must preserve this relationship across orchestrator restarts.

Contributing occurrences remain inspectable through retained records and references. Combined work must not silently replace all prior event information with only the latest payload. Model context remains bounded: an assignment can receive a compact description and references for further inspection instead of injecting every payload. Storage retention and payload-size limits remain separate policies; combining assignments does not make event storage unbounded.

Deduplication is separate from combining. Redelivery of the same identifiable occurrence must not become another distinct occurrence in either mode. Combining applies to distinct occurrences of the configured trigger, not arbitrary unrelated work or different triggers.

The configured mode must be visible to humans and agents managing the trigger under ADR 0047. Overflow, rejection, and other admission failures must have explicit outcomes. Individual mode must not silently switch to combined mode when its queue fills. Configuration changes must preserve an understandable accounting of already accepted occurrences; exact transition behavior remains to be specified.

Alternatives considered: Always process individually, which can accumulate redundant work; or always combine, which can remove necessary per-occurrence assignment boundaries. Per-trigger selection supports both workflows without requiring a general workflow engine.

Open details: Default mode when omitted; queue and group limits; overflow handling; batching latency; mode changes with pending work; interaction with failures and retries; and missed schedule occurrences during downtime.

Validation scenarios: Create separate assignments in individual mode; accumulate one pending group in combined mode; retain one follow-up during active work; inspect contributing events; deduplicate redelivery; recover grouping after restart; handle startup and arrival races; and report capacity exhaustion without silently changing modes.
