# Console

This document records how people work with Cubebot through the web console and installed PWA: the dedicated frontend stack and what it may and may not own, the conversation-first workspace, the rule that operations need a live connection while drafts survive connection loss, and in-app notifications with optional push. Backend authority stays in the Rust orchestrator throughout; the frontend presents, routes and adapts requests.

## Dedicated TanStack Start frontend with Bun and Vite

Accepted 2026-10-23.

**Decision.** Run a dedicated frontend server using the React and TypeScript TanStack Start stack, Bun and Vite. The web console and installed PWA are primary ways people work with Cubebot, and their architecture must support an exceptionally responsive, polished experience across desktop and mobile. The user selected this deployment model on 2026-10-23 and explicitly required exceptional speed, performance and visual quality. Bun is the frontend runtime and package tooling; Vite handles the framework's development and build integration. Exact versions and the production hosting adapter need qualification.

Usage: a person opens a room and sees an initial rendered view, then follows streaming agent activity, sends messages, inspects artifacts, and opens task or memory details without losing conversation position. The same application adapts to mobile navigation and installed PWA use. Drafts and disconnection behavior follow [Online console operations with local drafts](#online-console-operations-with-local-drafts); notifications follow [In-app notifications with optional push](#in-app-notifications-with-optional-push).

Ownership: the frontend owns presentation, routing, rendering, client interaction state and frontend-specific request adaptation. The Rust orchestrator keeps authoritative identity, permissions, task and agent state, memory, skills, storage, and Kubernetes and sandbox lifecycle operations. TanStack server functions may compose authorized orchestrator requests but must not become another implementation of these domain rules. The frontend gets no direct platform database access, Kubernetes lifecycle authority, installation encryption key or provider credential store.

The dedicated server is an additional deployed frontend process; [One orchestrator process](agents.md#one-orchestrator-process) still selects one Rust orchestrator process for backend application handling and Kubernetes reconciliation. Frontend restart must not cancel accepted agent work. Orchestrator unavailability must appear honestly in the UI instead of presenting cached data as live.

Rendering and data loading: use server rendering where it improves the initial experience, followed by efficient client navigation and live updates. Browser-dependent editors, terminals and live viewers can load when needed. Avoid fetching the same initial data twice during hydration or creating serial request chains for independent data. Exact per-route rendering policy, query integration, stream transport and ingress routing are engineering choices.

Authentication stays platform-wide ([Platform-wide built-in accounts and optional OpenID Connect](identity.md#platform-wide-built-in-accounts-and-optional-openid-connect)). Server-rendered requests must preserve verified user context and pass through orchestrator authorization. Personalized response caches and query state must not cross users or installations. Hydrated state must contain only data the client is authorized to receive. Frontend presentation and disabled controls do not replace backend access checks.

Performance is an acceptance requirement, not an inference from framework choice. Establish measured budgets for initial rendering, route navigation, input responsiveness, stream update latency, long-conversation scrolling, memory use and bundle size. Test representative mobile hardware and network conditions, large retained histories, and the 100-registered/25-working-agent workload of [Initial qualification at 100 registered and 25 working agents](packaging.md#initial-qualification-at-100-registered-and-25-working-agents) and [Mixed organizational workloads for initial qualification](packaging.md#mixed-organizational-workloads-for-initial-qualification). Separate model response latency from UI and platform latency.

Engineering direction: split code by route and heavy feature; bound rendered history and event buffers; batch streaming updates so they do not rerender the whole console; preserve scroll anchors; paginate large collections; and isolate live browser and desktop rendering from chat interaction. Local pending feedback can be immediate but must stay distinct from server acceptance. Do not optimistically claim success for consequential management operations or browser and desktop takeover.

Visual quality requires a coherent design system for typography, spacing, color, iconography and interaction states. Desktop layouts and mobile navigation must be intentionally designed. Keyboard navigation, focus, contrast, touch targets, reduced motion, readable long-form content, and loading, empty, error, paused, dreaming and disconnected states are part of the product contract. Choose a visual direction through concrete designs rather than assume a component library guarantees quality.

[Conversation-first workspace](#conversation-first-workspace) organizes the console around conversations, with contextual access to goals, artifacts, memory and skills.

**Rationale.** Serving a browser-rendered static frontend from the Rust orchestrator was considered. The user selected a dedicated TanStack Start server despite the additional deployment process, prioritizing the frontend experience and this stack.

**Open details.** Detailed navigation and visual direction; runtime adapter and version pins; typed client generation; query and rendering policy; streaming and ingress; session forwarding; caching; UI libraries; initial frontend replica counts and independent frontend scaling; measured performance thresholds; and desktop/mobile acceptance scenarios.

**Validation scenarios.** Render an authorized initial view; hydrate without duplicate requests or leaked state; keep typing and scrolling responsive during concurrent streaming; handle long histories with bounded memory; reconnect without duplicate messages; preserve drafts; enforce current access; maintain agent work through frontend restart; and demonstrate accessible, polished desktop and mobile flows.

References reviewed on 2026-10-23: [Bun's TanStack Start guide](https://bun.sh/guides/ecosystem/tanstack-start) documents production hosting options, and [TanStack selective SSR](https://tanstack.com/start/latest/docs/framework/react/guide/selective-ssr) documents per-route rendering control. [Core Web Vitals](https://web.dev/articles/vitals) supplies baseline loading, responsiveness and stability measurements; Cubebot-specific thresholds remain to be set. No runtime or frontend implementation had been benchmarked when this was accepted.

## Conversation-first workspace

Accepted 2026-10-23. Amended 2026-10-26: each agent has exactly one permanent conversation, which selecting the agent opens directly.

**Decision.** Organize the web console and installed PWA around conversations with agents and rooms. The user selected this direction on 2026-10-23. Each agent has exactly one permanent conversation ([One permanent conversation per agent](agents.md#one-permanent-conversation-per-agent)): selecting an agent opens its existing continuous chat, and there is no new-conversation action, conversation picker or alternate context for that agent. Goals, assignments, artifacts, memory and skills are accessible in the context of a conversation. Work overviews and fleet administration remain available as supporting destinations.

Usage: a person selects an agent or room, reads the conversation, sends a message, follows agent activity, and opens a related goal or artifact without losing their place. They can inspect an agent's memory, installed skills and visible dreaming results through contextual navigation, subject to the existing access rules.

Desktop direction: navigation for agents and rooms, a central conversation, and contextual detail panels. Panels open when useful rather than permanently consuming space. Exact navigation groups, widths and visual styling are design work.

Mobile direction: keep the same destinations and capabilities using a focused conversation view and navigable detail screens or sheets. Preserve drafts and conversation position when moving between them. Do not compress a desktop multi-column layout into a narrow viewport.

Ownership: frontend navigation and presentation compose orchestrator-authorized data ([Dedicated TanStack Start frontend with Bun and Vite](#dedicated-tanstack-start-frontend-with-bun-and-vite)). A room membership or visible message does not grant access to every linked artifact, agent memory, skill operation or management control; each destination uses the applicable resource authorization. Agent management and content permissions stay distinct: managing an agent does not by itself grant reading or sending in its conversation. Unified memory ([Unified memory per agent](memory.md#unified-memory-per-agent)) is agent-owned across the agent's conversation, rooms and tasks; the interface must not imply that opening a different view creates a separate memory pool.

Conversation history presents messages and relevant work updates. Goals and assignments keep their durable identities and inspectable state independently of their appearance in chat. Link progress, questions, outcomes and artifacts to the work that produced them. Detailed operational records can be inspected separately so concurrent activity does not overwhelm the conversation.

The conversation view must distinguish accepted messages, pending sends, queued work, active work and failures. Agent state and task state may differ because an agent can run several lanes ([Agent execution lanes](agents.md#agent-execution-lanes)). Switching conversations or closing a panel does not stop work. Messages queue, and active work is steered explicitly ([Queued messages with explicit steering](rooms.md#queued-messages-with-explicit-steering)); cancellation applies to selected work and its descendants ([Cancel selected work and its descendants](rooms.md#cancel-selected-work-and-its-descendants)).

Room routing and collaboration follow [Direct mentions and a tool-free room coordinator](rooms.md#direct-mentions-and-a-tool-free-room-coordinator), [Independent assignments for multiple room mentions](rooms.md#independent-assignments-for-multiple-room-mentions) and [Explicit agent messaging and delegation](rooms.md#explicit-agent-messaging-and-delegation). Dreaming results are visible as required by [Idle-only dreaming with visible status and results](memory.md#idle-only-dreaming-with-visible-status-and-results). Connectivity, drafts and notifications follow [Online console operations with local drafts](#online-console-operations-with-local-drafts) and [In-app notifications with optional push](#in-app-notifications-with-optional-push).

This decision does not select threading, a component library or a fixed pixel layout.

**Rationale.** A work-first inbox (goals and outcomes as the entry point) and a fleet-first dashboard (status and operations first) were considered. The user selected conversations as the primary entry point. Supporting work and fleet views still serve their respective workflows.

**What changed.** The original allowed several direct conversations per agent. [One permanent conversation per agent](agents.md#one-permanent-conversation-per-agent) replaced that on 2026-10-26: the agent's identity is the conversation identity, no conversation-creation operation remains, and the website opens directly into the agent's chat.

**Open details.** Navigation grouping and search; task conversation presentation; contextual panel behavior; deep links and mobile back navigation; visual design; and presentation of work controls.

**Validation scenarios.** Move between an agent's conversation and a room; inspect a linked goal or artifact and return to the same scroll position; preserve a draft while opening memory details; navigate equivalent mobile flows; distinguish an agent's concurrent tasks; handle inaccessible links honestly; and keep conversation input responsive under the qualification workload ([Initial qualification at 100 registered and 25 working agents](packaging.md#initial-qualification-at-100-registered-and-25-working-agents), [Mixed organizational workloads for initial qualification](packaging.md#mixed-organizational-workloads-for-initial-qualification)). Implementation and UI qualification were pending when this was accepted.

## Online console operations with local drafts

Accepted 2026-10-23.

**Decision.** Require an online connection for chat delivery and management operations, while preserving local message drafts during connection loss. The web console and mobile PWA can lose connectivity while a person is composing a message or managing agents; the client keeps unfinished writing without suggesting that disconnected actions reached the platform. The user selected this model on 2026-10-23. Offline message delivery queues, queued management changes and an offline conversation-reading feature are outside the initial design.

Usage: a user composing a message loses connectivity and keeps editing a local draft. The interface identifies the disconnected state. After reconnection and any required reauthentication, the user can send the draft. Reconnecting alone does not submit it or start agent work.

Ownership: the frontend owns local drafts and connection-state presentation. The orchestrator owns accepted messages, authoritative resource state and operation outcomes. Backend authorization is still required when an operation is submitted. Local draft storage does not confer permission to send into a conversation after access has changed.

Drafts must be associated with the installation, user and intended conversation so account switching or another installation cannot expose or misroute them. Cross-device draft synchronization is not selected.

The interface must distinguish an unsent draft, an in-flight submission, a server-accepted message and an unresolved submission outcome. If a connection drops after sending but before acknowledgement, the message may already have been accepted. Reconnection must reconcile that submission using a stable identity before offering or performing a retry that could duplicate work. This recovery is distinct from accepting new offline sends into a delivery queue.

Management controls require current connectivity. A previously visible state must not be presented as live after disconnection, and edits or cancellation requests must not be represented as applied before confirmation. Existing visible content may remain, labeled as stale; the initial design does not promise durable offline access to conversation history.

Live browser and desktop control follows [Live browser viewing and explicit human takeover](sandbox.md#live-browser-viewing-and-explicit-human-takeover) and [Desktop as a VMM device](sandbox.md#desktop-as-a-vmm-device). Disconnected input must not accumulate for replay, and reconnection must establish current control ownership and page state before further interaction. This decision is about the console's connection; it does not change how agents themselves continue while the orchestrator is unreachable ([Bounded continuation during disconnection](agents.md#bounded-continuation-during-disconnection)). During coordinated upgrades, a request that cannot be safely accepted gets a retryable maintenance response and the local draft is retained ([Coordinated maintenance upgrades](packaging.md#coordinated-maintenance-upgrades)).

**Rationale.** Cached offline reading with queued chat messages, or a broader queue of offline management operations, was considered. Online operations with drafts avoid those additional synchronization contracts while preserving unfinished writing.

**Open details.** Draft persistence mechanism, retention, size limits and cleanup including logout; attachment handling; connection detection; submission reconciliation; stale-state presentation; and refresh behavior after reconnection.

**Validation scenarios.** Preserve a draft through connection loss; avoid automatic sending on reconnect; isolate drafts by installation and user; deny sending after access is revoked; reconcile an acknowledgement lost after message acceptance; identify stale views; and prevent replay of disconnected browser or desktop input.

## In-app notifications with optional push

Accepted 2026-10-23.

**Decision.** Provide an in-app notification inbox and opt-in push notifications for supported browsers and installed PWAs. Autonomous work can finish, fail or need human input while the console is closed, so users need a durable place to find relevant updates and an optional way to be alerted outside the application. The user selected this model on 2026-10-23. Users control notification preferences for relevant agents, rooms and event categories. The console remains the source of current task and conversation state.

Usage: an agent needs an answer to continue a task. Cubebot creates a notification for the intended authorized recipient and, if enabled, sends a push alert linking to the relevant conversation or task. The user opens Cubebot, authenticates if necessary, and sees the current state before responding.

Ownership: a notification module within the orchestrator owns recipient selection, preferences, durable inbox records and delivery attempts. Domain modules supply meaningful events and references. A push adapter handles supported delivery services and subscription lifecycle. The frontend owns preference controls, device enrollment and inbox presentation. No separate deployed notification service is required.

Defaults prioritize direct questions and work needing attention, with completion notifications configurable. Ordinary agent-to-agent chatter does not produce a human notification for every message. Dreaming results stay inspectable ([Idle-only dreaming with visible status and results](memory.md#idle-only-dreaming-with-visible-status-and-results)) without a push for each memory change. Budget pauses notify under these recipient rules ([Preserve progress and pause on configured budget exhaustion](agents.md#preserve-progress-and-pause-on-configured-budget-exhaustion)).

Notification recipients and content must respect current access. A notification cannot grant access to its destination, and opening a link requires normal backend authorization. Delivery must account for permissions changing between event creation and dispatch. Notifications already displayed on a device cannot be assumed retractable, so push content needs an explicit privacy policy; full transcripts and secrets must not be copied into alerts by default.

Inbox records and push attempts are distinct. A failed, delayed, unsupported or disabled push path must not erase the in-app notification. Push acceptance does not establish that a person saw or read it. Delivery retries need stable notification identities and bounded attempts, without promising exactly-once display on every device.

Device subscriptions are associated with the correct installation and user. Logout, account suspension, permission changes and invalid subscriptions need defined cleanup and delivery behavior. The core console must stay usable when push is unavailable. Opening an alert does not authorize an offline reply or queued management action ([Online console operations with local drafts](#online-console-operations-with-local-drafts)).

**Rationale.** In-app notifications only, initially, was considered. Optional push was selected to support unattended work and mobile use, while the durable inbox remains when background delivery is unavailable.

**Open details.** Event taxonomy and categories; recipient rules; inbox retention and read state; preference defaults; aggregation, rate limits and quiet times; payload privacy; device lifecycle; push infrastructure and browser qualification; and delivery retry policy.

**Validation scenarios.** Create an authorized inbox notification; deliver optional push; respect opt-out; keep the inbox entry after push failure; suppress unauthorized content; enforce access when a link opens; handle duplicate delivery attempts; and prevent subscriptions from crossing user accounts.

## Drawn content runs in a sandbox with no network

Accepted 2026-11-04, from the user's request that everything that can be rendered is rendered.

**Decision.** A message's `html`, `svg`, `mermaid`, `markdown`, `csv`, `tsv` and `math` fences, and files of those kinds, show their result with their source one click away. HTML and SVG run in an iframe sandboxed with `allow-scripts` alone, whose page starts with a Content-Security-Policy that blocks every network request. The frame has an opaque origin, so it cannot reach the console, its cookies or the API; the policy keeps it from calling out or loading anything. This replaces the HTML file viewer's earlier `sandbox=""` with scripts off, since a page without its scripts is often not the page; external images staying off remains the rule, as in [Markdown messages](#conversation-first-workspace).

**Consequences.** Previews that need a CDN, a font service or an API do not work, and agents are told to inline everything. A framed page can still navigate itself, which no policy prevents; what it could send is only its own source. JSX and other source needing a build shows as code.

## Voice calls over the chat pipeline

Accepted 2026-11-05, from the user's request to talk to agents like a phone call. The user set the feature, the providers, the embedded models, the transport and the placement of the pipeline; the details below marked *chosen here* were not specified and are open to overrule.

**Decision.** A call is a normal conversation with speech at each end. The orchestrator turns a person's speech into a text message and the agent's text into speech. The agent never sees audio, and its image, tools, memory, context and permissions are untouched.

Every sentence a person finishes becomes an ordinary `chat_turns` row through the same send path as typed text, including the `conversation.send` check, so it joins the agent's context, memory and lanes as any message does. Each row also carries `call_id`. The conversation shows those rows as one message: `history` and the event stream return a `calls` row in their place, as a `Turn` whose `call` field holds the time, duration, providers, turn count and the turns themselves. The `calls` row draws its `sequence` from `chat_turns`' own sequence so the two share one order.

**Pipeline.** `apps/orchestrator/src/speech` is one module behind one `Stt` and one `Tts` trait, with embedded, OpenAI and ElevenLabs implementations. Both traits are blocking, because each stream owns one thread: one listens and one speaks, so a call costs nothing when silent beyond its model state, and an idle orchestrator pays only for models already resident. Silero always finds the end of speech, whichever provider transcribes, so turn-taking does not change with the provider and external providers need no streaming protocol. External STT therefore has no partial transcripts. Text is spoken sentence by sentence as the agent's `text_delta` events arrive.

**Barge-in.** Silero detecting speech tells the page to drop queued audio, stops synthesis and mutes the rest of that reply. *Chosen here:* the agent's turn is not cancelled and its full reply stays in the transcript, because chat's rule is that a new message queues and only an explicit Stop cancels work. The agent is not told how much of its reply was heard.

**Transport.** One WebSocket, `/api/agents/{id}/call`, authenticated as the desktop socket is, with the same origin check and a recheck every 30 seconds. The page sends 16 kHz mono PCM16 as binary frames from an AudioWorklet; JSON text frames carry state and transcripts; binary frames from the server carry a 4-byte little-endian sample rate followed by PCM16. There is no codec.

**Configuration.** Installation settings hold the STT provider, TTS provider and voice; an agent may override each. Voice is Male or Female, chosen at creation and editable later. Providers: embedded (default), OpenAI (`gpt-4o-mini-transcribe`, `gpt-4o-mini-tts`) and ElevenLabs (`scribe_v1`, `eleven_flash_v2_5`). The OpenAI key is the one the provider connection already holds, and agent grants on that connection apply. *Chosen here:* the ElevenLabs key is a credential on `speech_settings`, encrypted with the installation key and write-only like provider keys, because ElevenLabs is not an inference provider and adding it to `ProviderKind` would offer it as a model.

**Embedded models.** sherpa-onnx pinned to 1.13.x, since 2.0 has no espeak-ng frontend and cannot run Piper: `sherpa-onnx-streaming-zipformer-en-2023-06-26` int8, `silero_vad.onnx`, and Piper `en_US-lessac-medium` (Female) and `en_US-ryan-medium` (Male). Models download on first use into `speech-models` under the orchestrator's data directory (`CUBEBOT_STORAGE_DIRECTORY`), are checked against a pinned SHA-256 before use, and are never baked into an image. Once there they work offline. They load on the first call that needs them and stay resident.

*Chosen here:*

- Hanging up does not cancel the agent's work. A turn still unsettled stays reachable: the Call message says so and offers the usual Stop and Retry on it, because a `waiting` turn holds the main lane.
- Each turn in a call tells the model, through its instructions, that it is on a voice call and should answer briefly in plain speech. This is the only difference a model sees. Markdown it writes anyway is flattened and code blocks are skipped when spoken; the transcript keeps the original.
- A call with no turns is kept, as a phone log keeps a missed attempt.
- Calls left open by a restart are closed at startup, which is safe because one orchestrator is active ([One active orchestrator with automatic restart](agents.md#one-active-orchestrator-with-automatic-restart)).

**Rationale.** Keeping audio in the orchestrator leaves the sandbox boundary and every chat rule unchanged, and a model-free agent image stays small. Running each model through sherpa-onnx's safe wrapper means no `unsafe` in the orchestrator, as the repository requires. A transcript stored as turns rather than as a copy means there is one source for what was said, and the agent's context needs no call-specific handling.

**Consequences.** The orchestrator links a native library, its image grows by about 70 MB, and resident models cost it memory. It now does CPU work proportional to active calls, which it did not do before. Calls are not capped. The first embedded call waits for a download of several hundred MB, shown on the call screen. Echo control is the browser's: without it, an agent heard through a loudspeaker can interrupt itself.

**Open details.** Non-English embedded models; a limit on concurrent calls; telling the agent how much of a reply was heard; mute.

**Validation scenarios.** Real speech through the embedded models yields the expected text; a call produces one Call message whose turns reached the agent's context; speaking over the agent stops playback; refusing a person's `conversation.send` refuses their call; each external adapter sends its documented request; a tampered model download is refused.

## A calendar of the person's events and their agents' schedules

Accepted 2026-11-09, at the user's direction (docs/superpowers/specs/2026-11-09-calendar-design.md). The user set the scope, the noise rule, the time zone, recurring edits and editing; the details marked *chosen here* were not specified and are open to overrule.

**Decision.** The console has a Calendar that shows, on one timeline in the person's time zone, the events of the calendar connections they created (`connections.created_by`) and the routines of the agents they created (`agents.created_by`). Nobody sees another person's calendars or agents there, administrators included, which keeps [Installation-managed external account connections](identity.md#installation-managed-external-account-connections): a member who adds no connection sees only routines.

- **Events** come from Google, Microsoft 365 and CalDAV connections with Calendar switched on, through the same connector tools agents use ([Native connectors](identity.md#native-connectors)), called by the orchestrator with the connection's credential. Every calendar a connection lists is a source. Nothing is stored: each read asks the vendors, and repeating events come as their occurrences, exceptions applied. A connection or calendar that fails shows its problem on its source; the rest still shows.
- **Routines** show by their schedule, read in the routine's own zone: each upcoming run, and the scheduled runs that happened with their outcome. A routine with no schedule (webhook or connection event only) is not shown. A routine with more than 24 runs in one of the person's days shows that day as one digest with the count.
- **Writes.** Events are created, changed and deleted from the calendar; a change or deletion of a repeating event's occurrence applies to that occurrence or, as the person chooses, to all events of the series. The connection's level governs: a connection whose level does not offer the write is read-only. Routines are written through the routine endpoints; the calendar adds no routine API.
- **Time zone.** The profile's zone, else the browser's. Google and Microsoft writes are made in that zone; a CalDAV occurrence is named in the event's own zone.

*Chosen here:* a read covers at most 62 days; the orchestrator keeps no cache, and the console caches per range; calendar writes are not audited, as agents' tool calls are not; a change answers no content and the console reads the range again, because a series change moves many occurrences.

**Rationale.** Asking the vendors on each read keeps PostgreSQL free of anyone's calendar and needs no sync. Reusing the connector tools gives agents and people the same reads and writes, so a fix reaches both.

**Consequences.** A read is as slow as the slowest calendar and makes one vendor request per calendar (Google pages through 250 events at a time). Microsoft reads at most 500 events and CalDAV 100 occurrences per calendar and range.
