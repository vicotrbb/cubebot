# ADR 0054: In-app notifications with optional push

Status: Accepted

Date: 2026-10-23

Autonomous work can finish, fail, or require human input while the console is closed. Users need a durable place to find relevant updates and an optional way to receive alerts outside the application.

Decision: Provide an in-app notification inbox and opt-in push notifications for supported browsers and installed PWAs. The user selected this model on 2026-10-23. Users control notification preferences for relevant agents, rooms, and event categories. The console remains the source of current task and conversation state.

Usage: An agent needs an answer to continue a task. Cubebot creates a notification for the intended authorized recipient and, if enabled, sends a push alert linking to the relevant conversation or task. The user opens Cubebot, authenticates if necessary, and sees the current state before responding.

Ownership: A notification module within the orchestrator owns recipient selection, preferences, durable inbox records, and delivery attempts. Domain modules supply meaningful events and references. A push adapter handles supported delivery services and subscription lifecycle. The frontend owns preference controls, device enrollment, and inbox presentation. This does not require a separate deployed notification service.

Defaults should prioritize direct questions and work requiring attention, with completion notifications configurable. Ordinary agent-to-agent chatter does not produce a human notification for every message. Dreaming results remain inspectable under ADR 0025 without requiring a push for each memory change. Exact event categories, aggregation, rate limits, and quiet-time behavior remain to be specified.

Notification recipients and content must respect current access. A notification cannot grant access to its destination, and opening a link requires normal backend authorization. Delivery must account for permissions changing between event creation and dispatch. Notifications already displayed on a device cannot be assumed retractable, so push content needs an explicit privacy policy; full transcripts and secrets must not be copied into alerts by default.

Inbox records and push attempts are distinct. A failed, delayed, unsupported, or disabled push path must not erase the in-app notification. Push acceptance does not establish that a person saw or read it. Delivery retries need stable notification identities and bounded attempts, without promising exactly-once display on every device.

Device subscriptions are associated with the correct installation and user. Logout, account suspension, permission changes, and invalid subscriptions require defined cleanup and delivery behavior. The core console must remain usable when push is unavailable. Opening an alert does not authorize an offline reply or queued management action contrary to ADR 0053.

Alternative considered: In-app notifications only initially. Optional push is selected to support unattended work and mobile use while retaining a durable inbox when background delivery is unavailable.

Open details: Event taxonomy; recipient rules; inbox retention and read state; preference defaults; aggregation and quiet times; payload privacy; device lifecycle; push infrastructure and browser qualification; and delivery retry policy.

Validation scenarios: Create an authorized inbox notification; deliver optional push; respect opt-out; retain the inbox entry after push failure; suppress unauthorized content; enforce access when a link opens; handle duplicate delivery attempts; and prevent subscriptions from crossing user accounts.
