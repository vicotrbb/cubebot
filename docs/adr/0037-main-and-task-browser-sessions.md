# ADR 0037: Main browser session with separate task sessions

Status: Accepted

Date: 2026-10-23

An agent can run its main lane alongside bounded concurrent task lanes under ADR 0005. Browser work needs the same concurrency without one task navigating away from another task's page or redirecting its visual input.

Decision: Give the main lane its own browser session and allocate a separate session to each task that needs browser access, subject to resource limits. The user selected this model on 2026-10-23. Structured and visual actions within a lane address that lane's assigned session under ADR 0036.

Usage: The main lane fills out a form while two admitted tasks research separate websites. Each operates on its own pages and receives observations from its own session. A task navigating, closing a page, or sending keyboard input must not alter the main lane's browser target. Tasks that do not need a browser need no allocated browser session.

Ownership: The browser module owns session identity, task-to-session assignment, page ownership, and lifecycle. Scheduling owns admission against browser and agent resource limits. The runtime carries the execution identity into each browser request so the browser module can reject mismatched or stale references. Callers use the browser interface rather than managing browser processes directly.

Separate sessions must provide independent page state and input targeting. A shared desktop pointer or keyboard focus must not let a task's visual action affect a different session. The implementation may use browser contexts, separate processes, or another supported arrangement; the session contract is selected here, while the concrete browser and display layout remain open.

All sessions belong to the same agent and run in its tool environment under ADR 0033. This decision introduces no additional pods, per-task containers, agent identities, or independent workspace security. Sessions are an execution-coordination mechanism, not isolation between mutually untrusted users. Shared files and concurrent changes to the same external account still require coordination.

Browser concurrency is bounded by the existing lane limits and available resources. It does not guarantee a browser instance can always start. Unavailable capacity must lead to explicit waiting or a reported resource failure, not silent reuse of another lane's active session. The number of browser processes and their resource accounting remain implementation details.

Main-session allocation does not promise that a live browser process, open page state, or website login survives sleep or pod replacement. ADR 0007 still governs loss of in-memory execution state. [ADR 0038](0038-reuse-authorized-website-logins.md) permits reuse of website logins authorized for the agent and applicable to the task, without inheriting every cookie or credential from the main session. Profile persistence and task-session recovery and cleanup remain open.

Recovery must reconcile task and session identity before accepting further actions. Browser failure does not prove that a submitted external action failed; continuation must inspect recorded outcomes and re-observe before retrying. A stale session or page handle must not be silently rebound to a different task's browser.

Alternative considered: One shared browser session reserved exclusively by one lane at a time. That simplifies shared state but serializes browser work across otherwise concurrent tasks. The user selected separate main and task sessions.

Consequences: Cubebot needs per-session lifecycle tracking, bounded allocation, and reliable visual targeting. External-account conflicts remain possible even when pages are separate. Browser session isolation does not supersede network grants, application permissions, or workspace coordination.

Related control decision: [ADR 0039](0039-browser-live-view-and-takeover.md) supports live viewing and exclusive human takeover per session, with separate viewing and control permissions.

Open details: Browser context and process layout; visual input routing; profile persistence; login reuse mechanics under ADR 0038; allocation limits; task-session retention and cleanup; crash recovery; download naming; and control handoff mechanics under ADR 0039.

Validation scenarios: Main and task lanes browse concurrently without navigation interference; structured and visual actions address the same assigned session; stale or cross-task handles are rejected; browser capacity exhaustion is visible; closing one task session leaves others usable; resource cleanup respects active invocations; and browser restart does not replay an uncertain submission.
