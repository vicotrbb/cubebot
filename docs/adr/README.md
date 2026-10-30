# Cubebot architecture decisions

The [manifesto](../../MANIFESTO.md) defines the project intent. ADRs record consequential choices made during the architecture discussion. Earlier conversational recommendations remain proposals until explicitly selected.

The initial product-question sequence concluded with ADR 0080. Read the consolidated [architecture](../../ARCHITECTURE.md), [software specification](../design/software-specification.md), and [qualification plan](../design/qualification.md). They connect the accepted decisions and label additional engineering proposals; they do not silently promote those proposals to accepted ADRs.

Each ADR records context, alternatives, the selected decision, consequences, and scenarios needed to validate it. Use Proposed while discussing, Accepted after the user selects the choice, and Superseded when a later ADR replaces it. Record unresolved details explicitly. Do not treat silence as acceptance.

| ADR | Topic | Status |
| --- | --- | --- |
| [0001](0001-installation-and-tenancy.md) | Installation and tenancy | Accepted |
| [0002](0002-agent-ownership.md) | Agent ownership | Accepted |
| [0003](0003-administrator-content-access.md) | Administrator content access | Accepted |
| [0004](0004-unified-agent-memory.md) | Unified memory per agent | Accepted |
| [0005](0005-agent-execution-lanes.md) | Agent execution lanes | Accepted |
| [0006](0006-native-rust-agent-engine.md) | Native Rust agent engine | Accepted |
| [0007](0007-agent-idle-lifecycle.md) | Agent idle lifecycle | Accepted |
| [0008](0008-durable-storage.md) | Durable storage | Accepted |
| [0009](0009-agent-state-access.md) | Agent state access through the orchestrator | Partially superseded by 0083 |
| [0010](0010-orchestrator-process-structure.md) | One orchestrator process | Accepted |
| [0011](0011-orchestrator-availability.md) | One active orchestrator with automatic restart | Accepted |
| [0012](0012-agent-disconnection-behavior.md) | Bounded continuation during disconnection | Partially superseded by 0083 |
| [0013](0013-tool-autonomy.md) | Autonomous tool use within granted permissions | Accepted |
| [0014](0014-room-routing.md) | Direct mentions and a tool-free room coordinator | Accepted |
| [0015](0015-multiple-room-mentions.md) | Independent assignments for multiple room mentions | Accepted |
| [0016](0016-agent-messaging-and-delegation.md) | Explicit agent messaging and delegation | Accepted |
| [0017](0017-agent-initiated-creation.md) | Agent creation as a granted capability | Accepted |
| [0018](0018-created-agent-lifetime.md) | Persistent and task-bound created agents | Accepted |
| [0019](0019-created-agent-knowledge.md) | Fresh memory with an explicit briefing | Accepted |
| [0020](0020-memory-consolidation-and-source-history.md) | Preserve source history during memory consolidation | Accepted |
| [0021](0021-memory-creation.md) | Explicit and automatic memory creation | Accepted |
| [0022](0022-hybrid-memory-retrieval.md) | Hybrid memory retrieval | Accepted |
| [0023](0023-tool-initiated-memory-retrieval.md) | Tool-initiated memory retrieval | Accepted |
| [0024](0024-rolling-context-compaction.md) | Rolling context compaction | Accepted |
| [0025](0025-idle-dreaming-and-visible-results.md) | Idle-only dreaming with visible status and results | Accepted; admission line superseded by 0083 |
| [0026](0026-human-memory-editing.md) | Direct memory editing and conversational correction | Accepted |
| [0027](0027-evidence-based-memory-revision.md) | Revise human-corrected memories from new evidence | Accepted |
| [0028](0028-installation-managed-provider-connections.md) | Installation-managed provider connections | Accepted |
| [0029](0029-model-default-and-purpose-overrides.md) | Default agent model with optional purpose overrides | Accepted |
| [0030](0030-retry-selected-model-without-fallback.md) | Retry and wait for the selected model | Accepted |
| [0031](0031-direct-inference-and-propagated-authentication.md) | Direct provider inference with propagated authentication | Partially superseded by 0083 |
| [0032](0032-orchestrator-owned-provider-authentication-refresh.md) | Orchestrator-owned provider authentication refresh | Accepted |
| [0033](0033-separate-runtime-and-tool-containers.md) | Separate runtime and tool containers within each agent pod | Superseded by 0083 |
| [0034](0034-agent-software-installation.md) | Agent-directed software installation within grants | Accepted |
| [0035](0035-tool-network-access-defaults.md) | Public internet access with grants for private destinations | Accepted |
| [0036](0036-structured-and-visual-browser-interaction.md) | Structured browser tools with visual interaction | Accepted |
| [0037](0037-main-and-task-browser-sessions.md) | Main browser session with separate task sessions | Accepted |
| [0038](0038-reuse-authorized-website-logins.md) | Reuse authorized website logins across browser sessions | Accepted |
| [0039](0039-browser-live-view-and-takeover.md) | Live browser viewing and explicit human takeover | Accepted |
| [0040](0040-native-core-tools-and-mcp-integrations.md) | Native core tools with MCP integrations | Accepted |
| [0041](0041-local-and-remote-mcp-servers.md) | Local and remote MCP server support | Accepted |
| [0042](0042-administrator-managed-integration-configuration.md) | Administrator-managed integration configuration | Accepted |
| [0043](0043-installation-managed-external-account-connections.md) | Installation-managed external account connections | Accepted |
| [0044](0044-tool-allowlists-and-resource-restrictions.md) | Tool allowlists with supported resource restrictions | Accepted |
| [0045](0045-explicit-tool-search-and-on-demand-discovery.md) | Explicit tool search with on-demand discovery | Accepted |
| [0046](0046-scheduled-and-event-triggered-work.md) | Scheduled and event-triggered work | Accepted |
| [0047](0047-autonomous-trigger-management.md) | Trigger management within ordinary agent autonomy | Accepted |
| [0048](0048-per-trigger-occurrence-handling.md) | Configurable occurrence handling per trigger | Accepted |
| [0049](0049-platform-wide-human-authentication.md) | Platform-wide built-in accounts and optional OpenID Connect | Accepted |
| [0050](0050-invitations-and-optional-oidc-admission.md) | Invitations and optional OpenID Connect admission | Accepted |
| [0051](0051-built-in-roles-and-scoped-resource-grants.md) | Built-in roles with scoped resource grants | Accepted |
| [0052](0052-locally-managed-human-teams.md) | Human teams managed entirely in Cubebot | Accepted |
| [0053](0053-online-console-with-local-drafts.md) | Online console operations with local drafts | Accepted |
| [0054](0054-in-app-notifications-and-optional-push.md) | In-app notifications with optional push | Accepted |
| [0055](0055-reusable-skills-and-autonomous-skill-creation.md) | Reusable skills created by humans, agents, and dreaming | Accepted |
| [0056](0056-shared-installation-skill-library.md) | Shared installation skill library by default | Accepted |
| [0057](0057-collaborative-shared-skill-revisions.md) | Shared skill revisions by any agent or user | Accepted |
| [0058](0058-skill-imports-and-agent-skills-compatibility.md) | File and Git imports with Agent Skills compatibility | Accepted |
| [0059](0059-orchestrator-managed-file-storage.md) | Orchestrator-managed file storage on a persistent volume | Accepted |
| [0060](0060-native-storage-api.md) | Native Cubebot storage API | Accepted |
| [0061](0061-initial-production-qualification-scale.md) | Initial qualification at 100 registered and 25 working agents | Accepted |
| [0062](0062-mixed-organizational-qualification-workloads.md) | Mixed organizational workloads for initial qualification | Accepted |
| [0063](0063-bundled-and-external-postgresql.md) | Bundled PostgreSQL by default with external PostgreSQL support | Accepted |
| [0064](0064-coordinated-backup-and-restore.md) | Cubebot-coordinated backup and restore workflows | Accepted |
| [0065](0065-maintenance-window-backups.md) | Coordinated maintenance windows for backups | Accepted |
| [0066](0066-encrypted-credentials-with-separate-installation-key.md) | Encrypted credentials with a separate installation key | Accepted |
| [0067](0067-shared-goal-budgets-with-optional-cost-limits.md) | Shared goal budgets with optional cost limits | Accepted |
| [0068](0068-resumable-pause-on-budget-exhaustion.md) | Preserve progress and pause on configured budget exhaustion | Accepted |
| [0069](0069-dedicated-tanstack-start-frontend.md) | Dedicated TanStack Start frontend with Bun and Vite | Accepted |
| [0070](0070-conversation-first-workspace.md) | Conversation-first workspace | Accepted |
| [0071](0071-queued-messages-and-explicit-steering.md) | Queued messages with explicit steering | Accepted |
| [0072](0072-cancellation-of-delegated-work.md) | Cancel selected work and its descendants | Accepted |
| [0073](0073-contained-delegation-failures.md) | Contain delegated failures and let the parent adapt | Accepted |
| [0074](0074-postgresql-owned-agent-definitions.md) | PostgreSQL owns agent definitions and lifecycle intent | Accepted |
| [0075](0075-single-installation-namespace.md) | One namespace for the entire installation | Accepted |
| [0076](0076-standard-containers-and-optional-sandbox-runtimes.md) | Standard containers with optional sandbox runtimes | Accepted |
| [0077](0077-coordinated-maintenance-upgrades.md) | Coordinated maintenance upgrades | Accepted |
| [0078](0078-initial-provider-support.md) | Initial OpenAI, Anthropic, and compatible-endpoint support | Accepted |
| [0079](0079-explicit-deletion-for-durable-data.md) | Retain durable data until explicitly deleted | Accepted |
| [0080](0080-agent-completion-with-optional-human-acceptance.md) | Agent completion with optional human acceptance | Accepted |
| [0081](0081-installation-wide-default-connection-access.md) | Installation-wide connection access by default | Accepted |
| [0082](0082-one-permanent-conversation-per-agent.md) | One permanent conversation per agent | Accepted |
| [0083](0083-agent-centric-execution-in-a-sandbox-boundary.md) | Agent-centric execution in a sandbox boundary | Accepted; sandbox mechanism amended by 0084 |
| [0084](0084-in-house-vmm-as-the-agent-sandbox.md) | In-house VMM as the agent sandbox | Accepted |
| [0085](0085-desktop-as-a-vmm-device.md) | Desktop as a VMM device | Accepted |

Discussion will follow dependencies and adapt to answers: installation and trust; primary workflows and operating targets; agent identity and execution; conversations, rooms, tasks, and delegation; persistence and recovery; memory and context; provider and tool integration; console and mobile access; operations and validation.

For each consequential choice, compare meaningful alternatives and state a recommendation. Ask one focused question at a time. Record accepted choices before moving on. Prefer deep modules with small interfaces, clear ownership, and tests through the same interfaces callers use. Add adapters where requirements demonstrate variation, and use platform capabilities before adding custom machinery.

Accepted decisions will feed a system architecture and software specification covering usage, domain types and states, module ownership, interfaces, data flows, deployment, failure handling, and measurable acceptance criteria. Document changes are in scope; product implementation is a separate stage.
