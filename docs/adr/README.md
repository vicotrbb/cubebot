# Cubebot architecture decisions

The [manifesto](../../MANIFESTO.md) defines the project intent. These documents record the consequential choices the user accepted, one document per decision area. The [architecture](../../ARCHITECTURE.md) connects them into a system.

| Document | Covers |
| --- | --- |
| [Identity, access and credentials](identity.md) | installation and tenancy, ownership, human accounts, roles, grants and groups of people, provider and integration connections, the vault and credential encryption |
| [Agents and runtime](agents.md) | the native agent engine, its one conversation, lanes, idle lifecycle and definitions, the orchestrator's role and availability, models and providers, tools and MCP, routines and triggers, budgets, agent teams |
| [Sandbox and isolation](sandbox.md) | the sandbox boundary, the in-house VMM and the declared `kvm` and `container` runtimes, the desktop, software installation, network access and the browser |
| [Rooms and delegation](rooms.md) | room routing and mentions, agent messaging and delegation, steering, cancellation, failure containment and completion |
| [Memory, skills and files](memory.md) | agent memory, retrieval, compaction, dreaming and human correction, skills, durable and shared-file storage, retention |
| [Console](console.md) | the TanStack Start frontend, the conversation-first workspace, online operation with local drafts, notifications |
| [Packaging and operations](packaging.md) | the installation namespace, PostgreSQL deployment, qualification targets, backup, restore, maintenance and upgrades |

Each section states the decision as it stands today, with its rationale, open details and the scenarios that would validate it. Where a later decision replaced part of an earlier one, the later one wins and the section's **What changed** says what was replaced.

## Changing a decision

A change that contradicts an accepted decision needs the user's acceptance first. Then edit the decision's section in place: add an `Amended <date>` line, state the new decision, and record what it replaced under **What changed**. A new decision area gets a new section in the closest document. Record unresolved details explicitly; silence is not acceptance.

## Former ADR numbers

Until 2026-11-03 each decision was a numbered ADR file. Migration comments and older commits still cite those numbers; this table maps each to its section. Status is as the old index recorded it.

| ADR | Now | Former status |
| --- | --- | --- |
| 0001 Installation and tenancy | [Identity, access and credentials: Installation and tenancy](identity.md#installation-and-tenancy) | Accepted |
| 0002 Agent ownership | [Identity, access and credentials: Agent ownership](identity.md#agent-ownership) | Accepted |
| 0003 Administrator content access | [Identity, access and credentials: Administrator content access](identity.md#administrator-content-access) | Accepted |
| 0004 Unified memory per agent | [Memory, skills and files: Unified memory per agent](memory.md#unified-memory-per-agent) | Accepted |
| 0005 Agent execution lanes | [Agents and runtime: Agent execution lanes](agents.md#agent-execution-lanes) | Accepted |
| 0006 Native Rust agent engine | [Agents and runtime: Native Rust agent engine](agents.md#native-rust-agent-engine) | Accepted |
| 0007 Agent idle lifecycle | [Agents and runtime: Agent idle lifecycle](agents.md#agent-idle-lifecycle) | Accepted |
| 0008 Durable storage | [Memory, skills and files: Durable storage](memory.md#durable-storage) | Accepted |
| 0009 Agent state access through the orchestrator | [Agents and runtime: Agent state access through the orchestrator](agents.md#agent-state-access-through-the-orchestrator) | Partially superseded by 0083 |
| 0010 One orchestrator process | [Agents and runtime: One orchestrator process](agents.md#one-orchestrator-process) | Accepted |
| 0011 One active orchestrator with automatic restart | [Agents and runtime: One active orchestrator with automatic restart](agents.md#one-active-orchestrator-with-automatic-restart) | Accepted |
| 0012 Bounded continuation during disconnection | [Agents and runtime: Bounded continuation during disconnection](agents.md#bounded-continuation-during-disconnection) | Partially superseded by 0083 |
| 0013 Autonomous tool use within granted permissions | [Agents and runtime: Autonomous tool use within granted permissions](agents.md#autonomous-tool-use-within-granted-permissions) | Accepted |
| 0014 Direct mentions and a tool-free room coordinator | [Rooms and delegation: Direct mentions and a tool-free room coordinator](rooms.md#direct-mentions-and-a-tool-free-room-coordinator) | Accepted |
| 0015 Independent assignments for multiple room mentions | [Rooms and delegation: Independent assignments for multiple room mentions](rooms.md#independent-assignments-for-multiple-room-mentions) | Accepted |
| 0016 Explicit agent messaging and delegation | [Rooms and delegation: Explicit agent messaging and delegation](rooms.md#explicit-agent-messaging-and-delegation) | Accepted |
| 0017 Agent creation as a granted capability | [Agents and runtime: Agent creation as a granted capability](agents.md#agent-creation-as-a-granted-capability) | Accepted |
| 0018 Persistent and task-bound created agents | [Agents and runtime: Persistent and task-bound created agents](agents.md#persistent-and-task-bound-created-agents) | Accepted |
| 0019 Fresh memory with an explicit briefing | [Memory, skills and files: Fresh memory with an explicit briefing](memory.md#fresh-memory-with-an-explicit-briefing) | Accepted |
| 0020 Preserve source history during memory consolidation | [Memory, skills and files: Preserve source history during memory consolidation](memory.md#preserve-source-history-during-memory-consolidation) | Accepted |
| 0021 Explicit and automatic memory creation | [Memory, skills and files: Explicit and automatic memory creation](memory.md#explicit-and-automatic-memory-creation) | Accepted |
| 0022 Hybrid memory retrieval | [Memory, skills and files: Hybrid memory retrieval](memory.md#hybrid-memory-retrieval) | Accepted |
| 0023 Tool-initiated memory retrieval | [Memory, skills and files: Tool-initiated memory retrieval](memory.md#tool-initiated-memory-retrieval) | Accepted |
| 0024 Rolling context compaction | [Memory, skills and files: Rolling context compaction](memory.md#rolling-context-compaction) | Accepted |
| 0025 Idle-only dreaming with visible status and results | [Memory, skills and files: Idle-only dreaming with visible status and results](memory.md#idle-only-dreaming-with-visible-status-and-results) | Accepted; admission line superseded by 0083 |
| 0026 Direct memory editing and conversational correction | [Memory, skills and files: Direct memory editing and conversational correction](memory.md#direct-memory-editing-and-conversational-correction) | Accepted |
| 0027 Revise human-corrected memories from new evidence | [Memory, skills and files: Revise human-corrected memories from new evidence](memory.md#revise-human-corrected-memories-from-new-evidence) | Accepted |
| 0028 Installation-managed provider connections | [Identity, access and credentials: Installation-managed provider connections](identity.md#installation-managed-provider-connections) | Accepted |
| 0029 Default agent model with optional purpose overrides | [Agents and runtime: Default agent model with optional purpose overrides](agents.md#default-agent-model-with-optional-purpose-overrides) | Accepted |
| 0030 Retry and wait for the selected model | [Agents and runtime: Retry and wait for the selected model](agents.md#retry-and-wait-for-the-selected-model) | Accepted |
| 0031 Direct provider inference with propagated authentication | [Agents and runtime: Direct provider inference with propagated authentication](agents.md#direct-provider-inference-with-propagated-authentication) | Partially superseded by 0083 |
| 0032 Orchestrator-owned provider authentication refresh | [Identity, access and credentials: Orchestrator-owned provider authentication refresh](identity.md#orchestrator-owned-provider-authentication-refresh) | Accepted |
| 0033 Separate runtime and tool containers within each agent pod | [Sandbox and isolation: Agent-centric execution in a sandbox boundary](sandbox.md#agent-centric-execution-in-a-sandbox-boundary) | Superseded by 0083 |
| 0034 Agent-directed software installation within grants | [Sandbox and isolation: Agent-directed software installation within grants](sandbox.md#agent-directed-software-installation-within-grants) | Accepted |
| 0035 Public internet access with grants for private destinations | [Sandbox and isolation: Public internet access with grants for private destinations](sandbox.md#public-internet-access-with-grants-for-private-destinations) | Accepted |
| 0036 Structured browser tools with visual interaction | [Sandbox and isolation: Structured browser tools with visual interaction](sandbox.md#structured-browser-tools-with-visual-interaction) | Accepted |
| 0037 Main browser session with separate task sessions | [Sandbox and isolation: Main browser session with separate task sessions](sandbox.md#main-browser-session-with-separate-task-sessions) | Accepted |
| 0038 Reuse authorized website logins across browser sessions | [Sandbox and isolation: Reuse authorized website logins across browser sessions](sandbox.md#reuse-authorized-website-logins-across-browser-sessions) | Accepted |
| 0039 Live browser viewing and explicit human takeover | [Sandbox and isolation: Live browser viewing and explicit human takeover](sandbox.md#live-browser-viewing-and-explicit-human-takeover) | Accepted |
| 0040 Native core tools with MCP integrations | [Agents and runtime: Native core tools with MCP integrations](agents.md#native-core-tools-with-mcp-integrations) | Accepted |
| 0041 Local and remote MCP server support | [Agents and runtime: Local and remote MCP server support](agents.md#local-and-remote-mcp-server-support) | Accepted |
| 0042 Administrator-managed integration configuration | [Identity, access and credentials: Administrator-managed integration configuration](identity.md#administrator-managed-integration-configuration) | Accepted |
| 0043 Installation-managed external account connections | [Identity, access and credentials: Installation-managed external account connections](identity.md#installation-managed-external-account-connections) | Accepted; amended by 0086 |
| 0044 Tool allowlists with supported resource restrictions | [Agents and runtime: Tool allowlists with supported resource restrictions](agents.md#tool-allowlists-with-supported-resource-restrictions) | Accepted |
| 0045 Explicit tool search with on-demand discovery | [Agents and runtime: Explicit tool search with on-demand discovery](agents.md#explicit-tool-search-with-on-demand-discovery) | Accepted |
| 0046 Scheduled and event-triggered work | [Agents and runtime: Scheduled and event-triggered work](agents.md#scheduled-and-event-triggered-work) | Accepted; settled by 0087 |
| 0047 Trigger management within ordinary agent autonomy | [Agents and runtime: Trigger management within ordinary agent autonomy](agents.md#trigger-management-within-ordinary-agent-autonomy) | Accepted; settled by 0087 |
| 0048 Configurable occurrence handling per trigger | [Agents and runtime: Configurable occurrence handling per trigger](agents.md#configurable-occurrence-handling-per-trigger) | Accepted; settled by 0087 |
| 0049 Platform-wide built-in accounts and optional OpenID Connect | [Identity, access and credentials: Platform-wide built-in accounts and optional OpenID Connect](identity.md#platform-wide-built-in-accounts-and-optional-openid-connect) | Accepted |
| 0050 Invitations and optional OpenID Connect admission | [Identity, access and credentials: Invitations and optional OpenID Connect admission](identity.md#invitations-and-optional-openid-connect-admission) | Accepted |
| 0051 Built-in roles with scoped resource grants | [Identity, access and credentials: Built-in roles with scoped resource grants](identity.md#built-in-roles-with-scoped-resource-grants) | Accepted |
| 0052 Human teams managed entirely in Cubebot | [Identity, access and credentials: Human teams managed entirely in Cubebot](identity.md#human-teams-managed-entirely-in-cubebot) | Accepted |
| 0053 Online console operations with local drafts | [Console: Online console operations with local drafts](console.md#online-console-operations-with-local-drafts) | Accepted |
| 0054 In-app notifications with optional push | [Console: In-app notifications with optional push](console.md#in-app-notifications-with-optional-push) | Accepted |
| 0055 Reusable skills created by humans, agents, and dreaming | [Memory, skills and files: Reusable skills created by humans, agents, and dreaming](memory.md#reusable-skills-created-by-humans-agents-and-dreaming) | Accepted |
| 0056 Shared installation skill library by default | [Memory, skills and files: Shared installation skill library by default](memory.md#shared-installation-skill-library-by-default) | Accepted |
| 0057 Shared skill revisions by any agent or user | [Memory, skills and files: Shared skill revisions by any agent or user](memory.md#shared-skill-revisions-by-any-agent-or-user) | Accepted |
| 0058 File and Git imports with Agent Skills compatibility | [Memory, skills and files: File and Git imports with Agent Skills compatibility](memory.md#file-and-git-imports-with-agent-skills-compatibility) | Accepted |
| 0059 Orchestrator-managed file storage on a persistent volume | [Memory, skills and files: Orchestrator-managed file storage on a persistent volume](memory.md#orchestrator-managed-file-storage-on-a-persistent-volume) | Accepted |
| 0060 Native Cubebot storage API | [Memory, skills and files: Native Cubebot storage API](memory.md#native-cubebot-storage-api) | Accepted |
| 0061 Initial qualification at 100 registered and 25 working agents | [Packaging and operations: Initial qualification at 100 registered and 25 working agents](packaging.md#initial-qualification-at-100-registered-and-25-working-agents) | Accepted |
| 0062 Mixed organizational workloads for initial qualification | [Packaging and operations: Mixed organizational workloads for initial qualification](packaging.md#mixed-organizational-workloads-for-initial-qualification) | Accepted |
| 0063 Bundled PostgreSQL by default with external PostgreSQL support | [Packaging and operations: Bundled PostgreSQL by default with external PostgreSQL support](packaging.md#bundled-postgresql-by-default-with-external-postgresql-support) | Accepted |
| 0064 Cubebot-coordinated backup and restore workflows | [Packaging and operations: Cubebot-coordinated backup and restore workflows](packaging.md#cubebot-coordinated-backup-and-restore-workflows) | Accepted; amended 2026-11-07: S3-compatible bucket, restore into a fresh installation |
| 0065 Coordinated maintenance windows for backups | [Packaging and operations: Backups without pauses](packaging.md#backups-without-pauses) | Accepted; amended 2026-11-07: no pause, accepted inconsistency |
| 0066 Encrypted credentials with a separate installation key | [Identity, access and credentials: Encrypted credentials with a separate installation key](identity.md#encrypted-credentials-with-a-separate-installation-key) | Accepted |
| 0067 Shared goal budgets with optional cost limits | [Agents and runtime: Shared goal budgets with optional cost limits](agents.md#shared-goal-budgets-with-optional-cost-limits) | Accepted |
| 0068 Preserve progress and pause on configured budget exhaustion | [Agents and runtime: Preserve progress and pause on configured budget exhaustion](agents.md#preserve-progress-and-pause-on-configured-budget-exhaustion) | Accepted |
| 0069 Dedicated TanStack Start frontend with Bun and Vite | [Console: Dedicated TanStack Start frontend with Bun and Vite](console.md#dedicated-tanstack-start-frontend-with-bun-and-vite) | Accepted |
| 0070 Conversation-first workspace | [Console: Conversation-first workspace](console.md#conversation-first-workspace) | Accepted |
| 0071 Queued messages with explicit steering | [Rooms and delegation: Queued messages with explicit steering](rooms.md#queued-messages-with-explicit-steering) | Accepted |
| 0072 Cancel selected work and its descendants | [Rooms and delegation: Cancel selected work and its descendants](rooms.md#cancel-selected-work-and-its-descendants) | Accepted |
| 0073 Contain delegated failures and let the parent adapt | [Rooms and delegation: Contain delegated failures and let the parent adapt](rooms.md#contain-delegated-failures-and-let-the-parent-adapt) | Accepted |
| 0074 PostgreSQL owns agent definitions and lifecycle intent | [Agents and runtime: PostgreSQL owns agent definitions and lifecycle intent](agents.md#postgresql-owns-agent-definitions-and-lifecycle-intent) | Accepted |
| 0075 One namespace for the entire installation | [Packaging and operations: One namespace for the entire installation](packaging.md#one-namespace-for-the-entire-installation) | Accepted |
| 0076 Standard containers with optional sandbox runtimes | [Sandbox and isolation: Standard containers with optional sandbox runtimes](sandbox.md#standard-containers-with-optional-sandbox-runtimes) | Accepted; restored for the `container` runtime by 0084's 2026-11-02 amendment |
| 0077 Coordinated maintenance upgrades | [Packaging and operations: Coordinated maintenance upgrades](packaging.md#coordinated-maintenance-upgrades) | Accepted |
| 0078 Initial OpenAI, Anthropic, and compatible-endpoint support | [Agents and runtime: Initial OpenAI, Anthropic, and compatible-endpoint support](agents.md#initial-openai-anthropic-and-compatible-endpoint-support) | Accepted |
| 0079 Retain durable data until explicitly deleted | [Memory, skills and files: Retain durable data until explicitly deleted](memory.md#retain-durable-data-until-explicitly-deleted) | Accepted |
| 0080 Agent completion with optional human acceptance | [Rooms and delegation: Agent completion with optional human acceptance](rooms.md#agent-completion-with-optional-human-acceptance) | Accepted |
| 0081 Installation-wide connection access by default | [Identity, access and credentials: Installation-wide connection access by default](identity.md#installation-wide-connection-access-by-default) | Accepted |
| 0082 One permanent conversation per agent | [Agents and runtime: One permanent conversation per agent](agents.md#one-permanent-conversation-per-agent) | Accepted |
| 0083 Agent-centric execution in a sandbox boundary | [Sandbox and isolation: Agent-centric execution in a sandbox boundary](sandbox.md#agent-centric-execution-in-a-sandbox-boundary) | Accepted; sandbox mechanism amended by 0084 |
| 0084 In-house VMM as the agent sandbox | [Sandbox and isolation: In-house VMM as the agent sandbox](sandbox.md#in-house-vmm-as-the-agent-sandbox) | Accepted; amended 2026-11-02: declared `kvm` and `container` runtimes |
| 0085 Desktop as a VMM device | [Sandbox and isolation: Desktop as a VMM device](sandbox.md#desktop-as-a-vmm-device) | Accepted |
| 0086 A vault the desktop arbiter fills | [Identity, access and credentials: A vault the desktop arbiter fills](identity.md#a-vault-the-desktop-arbiter-fills) | Accepted |
| 0087 Routines | [Agents and runtime: Routines](agents.md#routines) | Accepted |
