# Agents and runtime

These decisions cover how an agent runs: its native Rust engine and permanent conversation, its execution lanes and idle lifecycle, how its definition and lifecycle intent are stored and reconciled, and how it reaches platform state through one active orchestrator, including while disconnected. They also cover the agent's tools and autonomy, agent-created agents, model selection and provider inference, MCP integrations and tool discovery, routines (scheduled and event-triggered work), and shared goal budgets. Where the sandbox, rooms, memory or identity decisions change these, the later decision wins and the section says so.

## Native Rust agent engine

Accepted 2026-10-22. Amended 2026-10-27: the agent, inside its sandbox, owns its loop, context, memory behavior, tools, lane scheduling and recovery journal ([Agent-centric execution in a sandbox boundary](sandbox.md#agent-centric-execution-in-a-sandbox-boundary)).

**Decision.** Build Cubebot's own Rust agent engine around model/provider adapters. The user selected this approach on 2026-10-22. Cubebot owns the agent loop; provider adapters supply inference. Supervising external agent engines such as Codex or Claude Code is outside the selected architecture.

A lane supplies a conversation and work item to the engine. The engine constructs context, requests a model response, handles requested tools through Cubebot's execution rules, incorporates results, and continues until completion, a limit, cancellation, or a defined pause. Results and progress retain their agent, conversation, task, and turn identities.

The model adapter interface handles request and response translation, streaming, model capabilities, authentication integration, usage reporting where available, and provider errors. It must preserve distinctions callers need for correct execution instead of pretending all providers behave identically. [Initial OpenAI, Anthropic, and compatible-endpoint support](#initial-openai-anthropic-and-compatible-endpoint-support) selects native OpenAI and Anthropic adapters plus configurable OpenAI-compatible endpoints for the first release.

The agent owns its loop, context assembly and compaction, memory behavior, tools, lane scheduling and recovery journal; it decides when to call a model, which tool to run, what to remember and what to retrieve. Within the agent, the engine owns turn progression and context assembly, memory behavior owns retrieval and memory changes (recorded in the orchestrator's ledger), and tool execution owns invocation and enforcement of the agent's configured tool policy. These are logical module responsibilities, not a requirement for separate processes, crates, or a generic plugin framework. Keep each interface small and introduce abstractions only for demonstrated variation.

Consequences: context maintenance, checkpoints, tool behavior, and memory integration are Cubebot responsibilities across providers. Each execution lane has its own working context while retaining the agent's shared memory and workspace (and, per [One permanent conversation per agent](#one-permanent-conversation-per-agent), the agent's single conversation). Provider-specific inference and account limits still constrain admission. Native sessions, child agents, or compaction behavior of external agent engines are not assumed capabilities.

Subscription authentication remains a desired integration where the provider supports the selected deployment and inference path. This decision does not establish access to any particular subscription, and authentication support must be verified separately. [Installation-managed provider connections](identity.md#installation-managed-provider-connections) makes provider connections installation-managed, with explicit agent-use grants and no separate personal-connection ownership category. This decision also does not decide whether to use a Rust library internally for inference or agent-loop utilities.

**Rationale.** Owning the loop keeps reasoning, tool execution, context construction and recovery under Cubebot's control across providers. Alternatives considered: supervising existing engines would reuse their reasoning and tool loops but delegate control over important runtime behavior. Supporting both native and external engines would require two execution paths and different capability contracts. The user chose the native engine alone.

**What changed.** The original split responsibilities across orchestrator modules: memory owned retrieval and validated memory changes, tool execution owned authorization and invocation, and scheduling owned lane admission and budgets. [Agent-centric execution in a sandbox boundary](sandbox.md#agent-centric-execution-in-a-sandbox-boundary) moved memory behavior, tool execution and lane scheduling into the agent. The orchestrator no longer runs turns, schedules lanes, authorizes individual tool calls or decides memory policy; budgets are checked from ledger usage reports (see [Shared goal budgets with optional cost limits](#shared-goal-budgets-with-optional-cost-limits)).

**Open details.** Exact adapter types, models, protocol subsets, and authentication qualification. Detailed recovery and acceptance criteria.

**Validation scenarios.** A tool call followed by a final response; multiple lanes with separate working contexts and shared agent memory; provider stream failure; unavailable capabilities; cancellation during a tool operation; context pressure; and restart after an action whose outcome is uncertain.

## One permanent conversation per agent

Accepted 2026-10-26.

**Decision.** Every agent has exactly one permanent conversation, always. Its context is managed within that single conversation. The user explicitly corrected the MVP's multiple-conversation model on 2026-10-26.

Selecting an agent opens its existing continuous chat. There is no new-conversation action, conversation picker, independent session, or alternate context for that agent. Context-window management does not create a new conversation or erase retained history. Restarting, sleeping or editing an agent does not reset the conversation.

This supersedes earlier references to multiple direct conversations belonging to one agent in [Unified memory per agent](memory.md#unified-memory-per-agent), [Conversation-first workspace](console.md#conversation-first-workspace) and the first text-chat prototype. Agent management and content permissions remain distinct. One shared context does not authorize a person to read or send messages without the applicable content permission. Installation administrators retain their accepted default content access ([Administrator content access](identity.md#administrator-content-access)).

Implementation: the agent's identity is the conversation identity. A database view derives exactly one conversation from every agent, including a newly created agent that has never been opened. Message foreign keys refer to that same identity. No conversation-creation operation remains.

Migration: combine prior histories in their existing message sequence order without changing message/result identities. Retain former conversation IDs, each turn's source and each grant's original scope as migration evidence, not active histories. Old message references resolve to the permanent conversation after current authorization checks, preserving accepted-request retries without creating an alternate context. Preserve permissions for a single prior history. If several histories had separate audiences, revoke their old grants and require an administrator to approve access to the combined transcript. Preserve a prior initiator's rights only if that person already had access to every source history. Never union private audiences and silently expose their histories to each other.

**Rationale.** An agent is one continuous colleague, not a set of sessions; one conversation keeps its context, history and inference in one place. Migration fails closed rather than exposing private histories to each other.

**Open details.** None recorded.

**Validation scenarios.** New agents already have one conversation; concurrent openings and restarts return the same identity; additional conversation creation is unavailable; different authorized participants use the same stream; management alone does not grant content access; migration retains history and fails closed on different audiences; the website opens directly into the chat; inference context comes from that permanent history.

## Agent execution lanes

Accepted 2026-10-22. Amended 2026-10-26: task lanes use the agent's one permanent conversation rather than a separate conversation ([One permanent conversation per agent](#one-permanent-conversation-per-agent)). Amended 2026-10-27: lanes are a scheduling rule inside the agent loop, not an orchestrator admission rule, and the runtime and tools share one sandbox ([Agent-centric execution in a sandbox boundary](sandbox.md#agent-centric-execution-in-a-sandbox-boundary)). Amended 2026-11-04, at the user's direction: a turn of a task from the Tasks page ([Tasks and the task orchestrator](rooms.md#tasks-and-the-task-orchestrator)) runs in a task lane, one per task and counted within the agent's limit. Such a turn may use the desktop, one turn at a time: it holds the desktop from its first desktop call until it ends, and the main lane takes the same hold. A delegated task still never uses the desktop.

**Decision.** Adopt the OpenBot-style hybrid execution model described in the architecture discussion. The user selected this model on 2026-10-22. Lanes are scheduled by the agent inside its own loop.

- Each agent has one main lane shared by normal chat, room work, and external messaging. A lane is a durable queue that admits one active turn at a time; it is not a shared conversation transcript.
- Each task has its own lane and its own working context for its assignment. A task lane admits one active turn at a time and can run beside the main lane and other task lanes. No lane creates another chat session: the agent has one permanent conversation.
- The default task concurrency limit is two per agent, configurable from one to four. The main lane is outside that task count. Thus the default allows one main turn plus two task turns, subject to provider capabilities and installation capacity.
- Provider and account concurrency restrictions can reduce execution to one turn at a time. Under [Native Rust agent engine](#native-rust-agent-engine), Cubebot enforces these constraints around model inference; it does not delegate execution to an external provider runtime.
- Installation-wide admission also limits aggregate concurrent execution. When capacity is scarce, eligible queued human messages take priority, followed by queue age. The numerical installation limit and capacity configuration remain open.
- Different agents execute independently subject to shared capacity limits. Room work occupies the assigned agent's main lane, not every agent in the installation.

Usage: while an agent executes a room turn, its normal chat waits in the main lane. Two task lanes may still execute. A third task waits for capacity. If its provider only permits one turn, every other lane waits instead.

All lanes retain the same agent identity, memory ownership, and workspace. They do not become separate agents or acquire separate sandboxes merely because they are concurrent. Consistent with the reference model, there is no automatic per-task filesystem isolation. The runtime and its tools run together in the agent's one sandbox, with no per-lane containers. Shared workspace writes require explicit coordination. Task sessions do not concurrently control the agent's shared desktop; [Main browser session with separate task sessions](sandbox.md#main-browser-session-with-separate-task-sessions) assigns them separate browser sessions with independent action targeting, subject to capacity limits, and [Structured browser tools with visual interaction](sandbox.md#structured-browser-tools-with-visual-interaction) supports structured and visual actions in the same browser session.

Memory mutations follow the reference model's per-turn staging and successful-completion commit, with conflict checks. This does not roll back filesystem changes or external actions from a failed turn. Durable staging, crash reconciliation, and interaction with dreaming remain implementation design questions.

Ownership: the agent loop owns lane scheduling and concurrency limits. Provider adapters report execution capabilities. The agent owns memory behavior, and the orchestrator's ledger records the resulting memory records. Tool and workspace coordination must have explicit ownership in the module design. A lane remains reserved while a start outcome is uncertain until recovery resolves it; a timeout alone is not permission to duplicate execution.

Routine occurrences are ordinary turns in the permanent conversation, so these lanes, retries and cancellation apply to them unchanged ([Routines](#routines)). [Queued messages with explicit steering](rooms.md#queued-messages-with-explicit-steering) selects queued ordinary messages and explicit steering at safe execution boundaries. [Explicit agent messaging and delegation](rooms.md#explicit-agent-messaging-and-delegation) defines Cubebot messaging and delegation semantics; external provider-native child agents remain outside the native engine architecture.

Reference: OpenBot revision `cfee2971f29cdb7c16272e60ae912de7fbb30693`. This decision adopts the described execution design, not a blanket commitment to every upstream implementation detail.

**Rationale.** The hybrid preserves ordered execution within each lane and bounded parallelism across tasks. Alternatives considered: one active turn for the entire agent simplifies shared-state ownership but prevents parallel task execution. Arbitrary concurrent turns without lane serialization expose more ordering and coordination problems.

**What changed.** The original made scheduling (an orchestrator module) own lane admission and the memory module own mutation validation and commit; [Agent-centric execution in a sandbox boundary](sandbox.md#agent-centric-execution-in-a-sandbox-boundary) moved both into the agent loop. The original placed runtime and tool execution in separate containers within a shared agent pod; that was replaced by one sandbox per agent holding the runtime and its tools together. The original gave each task "its own lane and conversation context"; [One permanent conversation per agent](#one-permanent-conversation-per-agent) rules out any alternate conversation or session, so a task lane keeps its own working context for its assignment but uses the agent's single conversation.

**Open details.** Scheduling fairness under sustained human traffic; lane yielding and interruption mechanics; how the agent loop schedules concurrent lanes; installation capacity and its numerical limit; workspace and browser coordination; durable recovery; and background memory maintenance.

**Validation scenarios.** Main turn plus two tasks; excess task queued; sequential turns within one lane; provider-enforced serialization; unrelated agent progress during room work; concurrent memory edits; conflicting workspace operations; uncertain provider start; and recovery without duplicate task execution.

## Agent idle lifecycle

Accepted 2026-10-22. Amended 2026-10-27 and 2026-11-02: a running agent has one sandbox (a `cubebot-sandbox` KVM guest or the agent image as a container) rather than a pod split into containers ([Agent-centric execution in a sandbox boundary](sandbox.md#agent-centric-execution-in-a-sandbox-boundary), [In-house VMM as the agent sandbox](sandbox.md#in-house-vmm-as-the-agent-sandbox)). Amended 2026-11-03: agents are always on, and sleeping is an idle status that does not stop the sandbox.

**Decision.** Agents are always on. The user selected this model on 2026-11-03, replacing sleep-by-default. An agent's sandbox, and with it its desktop and whatever it installed or opened there, runs from creation until a person stops or retires the agent. An agent idle past its delay (ten minutes by default, configurable per agent) is *sleeping*: a status the console shows, not a stopped sandbox. While it sleeps it dreams, consolidating and improving its own memories ([Idle-only dreaming with visible status and results](memory.md#idle-only-dreaming-with-visible-status-and-results)), which is how a bounded context window holds an unbounded history. Any new work ends the sleep.

Each running agent has one sandbox (on Kubernetes, one pod and one workspace PVC; on Docker, one container). Stopping, which only a person requests, stops the agent's sandbox after preserving durable state. It preserves identity, configuration, the conversation, memory, task state, and persistent workspace data. Starting recreates execution from that state. It does not promise preservation of arbitrary in-memory processes, open terminal jobs, or live browser sessions.

Messages, scheduled work, and accepted external events start a stopped agent. [Scheduled and event-triggered work](#scheduled-and-event-triggered-work) and [Routines](#routines) make schedules and webhooks sources of durable work. Dreaming is scheduled, budgeted work that runs while the agent sleeps. Under [Idle-only dreaming with visible status and results](memory.md#idle-only-dreaming-with-visible-status-and-results), dreaming runs only when all execution lanes are idle and yields safely to arriving work; its status and results are visible in the console. Being idle does not require continuous inference. Always on means maintaining the execution environment, not continuously calling a model or bypassing capacity limits.

Sleep applies to the entire agent, including all execution lanes from [Agent execution lanes](#agent-execution-lanes). A quiet main lane is insufficient if another task is active or work is queued, waiting or delegated. A stop must not discard active work, pending durable writes, or unresolved action outcomes. The treatment of long-running background tools and safe checkpoints will be specified with execution recovery.

Ownership: the orchestrator retains work while a stopped agent is absent and starts and stops sandboxes. The native engine reports execution state and completes the required checkpoint or drain before a stop. The durable storage design must support access to retained state independently of whether the sandbox is running.

Consequences: every agent holds its sandbox's resources while it exists, so an installation's capacity bounds how many agents it keeps; stopping is how a person releases them. Startup latency is part of the experience only after a stop. The console must distinguish working, idle, sleeping (and dreaming), stopped, starting, and unavailable states. Always on expresses desired availability, not a guarantee against node failures or capacity shortages.

[Persistent and task-bound created agents](#persistent-and-task-bound-created-agents) separates lifetime from idle status. A task-bound agent can sleep while eligible for work; once retired, it cannot start for work or dreaming.

**Rationale.** An agent is meant to be lived with like a person at their own computer: what it installed and opened stays where it left it, and its desktop is always there to watch. Sleep-by-default made a new agent's desktop a black screen until a message woke it, and lost its open windows every idle spell. Alternatives considered: sleep by default with per-agent keep-running (the 2026-10-22 decision) reduces idle resource use but breaks that model and adds startup latency to every first message; removing stop entirely leaves no way to release resources short of retirement.

**What changed.** On 2026-11-03 sleep-by-default became always on: new agents are created running, the stored `sleeping` intent was renamed `stopped` (and running agents left in it were started), the `wake` and `sleep` lifecycle actions became `start` and `stop`, and `sleeping` became the activity of a running agent idle past its delay with nothing queued, waiting or delegated. The per-agent `idle_policy` field (`sleep` or `keep_running`) remains in the API and has no effect; no idle shutdown was ever implemented. Before that, the original gave each running agent a dedicated pod and had the orchestrator coordinate pod lifecycle. Under [Agent-centric execution in a sandbox boundary](sandbox.md#agent-centric-execution-in-a-sandbox-boundary) and [In-house VMM as the agent sandbox](sandbox.md#in-house-vmm-as-the-agent-sandbox), the unit is one sandbox per agent, run as a Kubernetes pod or a Docker container behind one `Sandbox` interface; sleeping and stopping the agent stop that sandbox.

**Open details.** Idle timeout; installation resource limits; wake latency targets; startup admission and retry; wake coalescing; sleep/wake races; background process handling; dreaming cadence and budgets; checkpoint format; and Kubernetes reconciliation mechanics.

**Validation scenarios.** A new agent's sandbox starts without a message; an agent idle past its delay reads as sleeping while its sandbox keeps running, and dreams; a message to a sleeping agent is taken without a start; a message starts a stopped agent; concurrent start requests do not create competing executors; an active task prevents sleep; a message arriving during a stop is retained and processed; and startup failure preserves queued work.

## PostgreSQL owns agent definitions and lifecycle intent

Accepted 2026-10-23. Amended 2026-10-28 and 2026-11-02: lifecycle reconciliation goes through a `Sandbox` interface with Kubernetes and Docker implementations ([In-house VMM as the agent sandbox](sandbox.md#in-house-vmm-as-the-agent-sandbox)).

**Decision.** Store authoritative agent definitions and desired lifecycle state in PostgreSQL. Users and authorized agents manage them through Cubebot. The orchestrator reconciles those records into sandbox resources (on Kubernetes, one pod and one workspace PVC per agent; cluster-less, Docker containers) and records observed status. No custom Kubernetes Agent resource is required in the initial architecture. The user selected this model on 2026-10-23.

Usage: a user creates an agent through the console. The orchestrator validates permissions and configuration and durably records the definition. When lifecycle policy and pending work require execution, the lifecycle module provisions or reconciles the agent's sandbox resources. Delayed scheduling or failed provisioning appears as status on the existing agent; it does not lose the definition or falsely report a running agent.

Ownership: agent-domain rules own definitions, grants, revisions, and lifecycle intent. Work routing determines which agents need to run under those rules. The lifecycle module translates desired state into supported resource specifications, performs cluster or Docker operations, and observes their outcomes. Kubernetes (or Docker) remains authoritative about actual resource state. PostgreSQL records that observation with enough identity and freshness information to distinguish it from desired state. Each Docker container is recorded as a pod attempt with its own epoch, so sessions, drains and recovery rules match Kubernetes.

An agent has a stable Cubebot identity independent of a pod name, Kubernetes object UID, or runtime incarnation. Ordinary pod replacement and idle shutdown preserve agent identity, memory, history, pending work, and the persistent workspace under existing decisions. Running, sleeping, retired, and provisioning-blocked states must reflect their distinct domain meanings; absence of a pod alone does not mean an agent was deleted.

Accepting a definition and applying it to Kubernetes are separate operations. Persist desired changes before attempting their external effects. Track revision and operation outcomes so retries, lost acknowledgements, and process restarts can converge without creating competing executors or losing accepted intent. The exact reconciliation queue and Kubernetes workload type remain engineering choices. A PostgreSQL transaction cannot atomically commit a Kubernetes API operation.

On restart, observe existing managed resources and reconcile pending intent before replacing uncertain execution. A missing or failed pod is not proof that previously issued external actions failed. Replacement remains subject to exclusive execution ownership, disconnected-agent behavior, and durable recovery under [One active orchestrator with automatic restart](#one-active-orchestrator-with-automatic-restart) and [Bounded continuation during disconnection](#bounded-continuation-during-disconnection). Database unavailability must not cause the controller to interpret agents as deleted or perform destructive cleanup from an empty or incomplete read.

Manual edits to managed Kubernetes fields do not become accepted Cubebot configuration. Reconciliation should restore Cubebot-owned fields according to current intent, using the defined safe update procedure. Preserve fields owned by Kubernetes or approved admission machinery. Unexpected ownership conflicts must be visible. Direct cluster edits are not a second configuration interface.

Record and validate installation and agent ownership for managed resources. Do not adopt or delete resources solely because a convenient name or label matches. Sleeping, retiring, or replacing a pod does not authorize deleting its retained workspace. Resource cleanup and storage retention must follow their explicit policies, including after backup restoration.

Related decisions: [Durable storage](memory.md#durable-storage) and [Agent state access through the orchestrator](#agent-state-access-through-the-orchestrator) define durable state and orchestrator-mediated access; [One orchestrator process](#one-orchestrator-process) places reconciliation in the single Rust backend process; [Agent idle lifecycle](#agent-idle-lifecycle) defines idle lifecycle; [Persistent and task-bound created agents](#persistent-and-task-bound-created-agents) defines task-bound retirement; [Cubebot-coordinated backup and restore workflows](packaging.md#cubebot-coordinated-backup-and-restore-workflows) and [Coordinated maintenance windows for backups](packaging.md#coordinated-maintenance-windows-for-backups) define coordinated backup and restore. [One namespace for the entire installation](packaging.md#one-namespace-for-the-entire-installation) places the installation's namespaced resources in one dedicated namespace, with workload-specific permissions and network controls.

**Rationale.** Keeping configuration, permissions, and lifecycle intent under one application authority avoids two sources of truth. Alternative considered: a Kubernetes custom Agent resource could make agent definitions declarative through kubectl and GitOps, while PostgreSQL retained application history. The user selected Cubebot-managed definitions in PostgreSQL. Supporting that alternative later would require an explicit ownership contract, not bidirectional synchronization by default.

**What changed.** The original reconciled agent records into Kubernetes resources only. [In-house VMM as the agent sandbox](sandbox.md#in-house-vmm-as-the-agent-sandbox) put that behind a `Sandbox` interface (create, start, stop, exec channel, workspace and status) implemented by Kubernetes and by Docker for cluster-less installs, with Docker containers recorded as attempts.

**Open details.** Kubernetes workload objects; resource ownership and field management; scoped RBAC; reconciliation scheduling and backoff; execution fencing; configuration activation and disruptive-change activation; maintenance controls; storage provisioning and cleanup; and restore handling for pre-existing resources.

**Validation scenarios.** Create while the Kubernetes API is unavailable; recover after creation succeeds but its response is lost; preserve an agent across pod replacement; converge after orchestrator restart; show pending and observed revisions honestly; handle manual drift without overwriting unrelated fields; refuse ambiguous resource adoption; and preserve workspaces and existing agents during database failure.

## Agent state access through the orchestrator

Accepted 2026-10-23. Amended 2026-10-27: the agent, not an orchestrator module, owns memory behavior, and work reporting no longer depends on execution leases ([Agent-centric execution in a sandbox boundary](sandbox.md#agent-centric-execution-in-a-sandbox-boundary)).

**Decision.** Agents access durable platform state through authenticated orchestrator operations. The user selected this model on 2026-10-23. Agents receive no platform database credentials and do not depend on its schema. This interface remains the only path to platform state.

Usage: an agent requests relevant memories, sends a message, or records task progress through the corresponding orchestrator interface. For example, sending a message expresses the recipient and content; the messaging module owns permission checks, durable recording, duplicate handling, and delivery scheduling. The agent writes results, memory changes and usage back to the ledger through idempotent operations.

Ownership: orchestrator modules own authorization, valid durable state transitions, and database transactions for their respective domains; database details stay inside their implementations. As ledger, the orchestrator durably records what humans and recovery need: agent definitions, conversations, messages, results, memory records, usage and audit. The agent owns its loop, tool-loop progression and memory behavior, and reports results through those interfaces.

An agent's authenticated identity constrains its operations. Sharing an installation does not authorize access to every agent's records. A sandbox holds one agent-scoped credential for its lifetime; revoking that credential stops its ledger writes.

Consequences: durable state operations require orchestrator availability and add a network exchange. A lost response may leave the caller uncertain whether an operation committed. Contracts must support safe retries and reconciliation rather than assume a network timeout means failure. Access mediation does not imply one orchestrator replica or one database connection.

Scope: this decision concerns Cubebot's structured platform state. It does not route every model request, terminal command, browser operation, or workspace file access through the orchestrator. [Direct provider inference with propagated authentication](#direct-provider-inference-with-propagated-authentication) selects direct provider inference from agents with orchestrator-propagated authentication. Local caches, if justified later, cannot silently become competing authoritative stores. [Bounded continuation during disconnection](#bounded-continuation-during-disconnection) lets agents keep working during disconnection, using a durable local journal and subsequent orchestrator reconciliation; agents still have no direct platform database access.

**Rationale.** A small domain-oriented interface hides persistence and coordination rules from agents. It keeps the implementation of those rules local and avoids distributing database credentials, schema knowledge, and connection management across agents. Alternative considered: direct PostgreSQL access from agents with scoped credentials would avoid the intermediary but distribute persistence knowledge and enforcement responsibilities. The user selected orchestrator-mediated access instead.

**What changed.** The original made the orchestrator's memory module own validated commits to shared agent memory, and required work assigned to an execution to remain valid when it reported progress, through lease and revocation mechanisms to be designed with recovery. [Agent-centric execution in a sandbox boundary](sandbox.md#agent-centric-execution-in-a-sandbox-boundary) gave memory behavior to the agent (the ledger records the memory records) and removed execution leases for reporting; authority is the agent's long-lived credential. Execution leases were accordingly dropped from the open details.

**Open details.** Transport, authentication and credential lifecycle, exact wire contracts and identifiers, idempotency contracts and retry semantics, streaming and batching, backpressure, and connection recovery.

**Validation scenarios.** Authorized memory retrieval; rejected cross-agent access; duplicate message submission after a lost acknowledgement; a stale or revoked agent credential reporting progress; orchestrator outage during a task; and recovery without duplicate durable transitions or falsely acknowledging unsaved work.

## One orchestrator process

Accepted 2026-10-23. Amended 2026-10-27 and 2026-10-28: the orchestrator is ledger and switchboard and no longer schedules lanes; lifecycle reconciliation drives sandboxes on Kubernetes or Docker ([Agent-centric execution in a sandbox boundary](sandbox.md#agent-centric-execution-in-a-sandbox-boundary), [In-house VMM as the agent sandbox](sandbox.md#in-house-vmm-as-the-agent-sandbox)).

**Decision.** Run application handling and sandbox lifecycle management in one Rust orchestrator process. The user selected this model on 2026-10-23. Do not deploy a separate Kubernetes controller process for the initial architecture.

The process contains modules for client and agent interfaces, authorization, rooms and messaging, work routing, the durable ledger (including memory records), and sandbox lifecycle reconciliation. These modules retain their own rules and interfaces. One process does not require one undifferentiated implementation, one thread, or one global lock. The orchestrator does not run turns, schedule lanes, authorize individual tool calls or decide memory policy.

Usage: a message is durably accepted, routing determines that its agent needs to run, and the lifecycle module reconciles the desired running state. When the agent's sandbox becomes ready and authenticates, the agent can receive its work. Failed or delayed sandbox creation does not discard the accepted message.

Ownership: domain modules own permissions and durable state transitions. The lifecycle module owns translation of desired agent state into Kubernetes or Docker operations and observed status, behind one `Sandbox` interface. Its implementation contains the backend-specific details, so messaging and memory callers do not need to construct pod specifications or understand Kubernetes request ordering. Network and reconciliation work must not block unrelated request handling.

Consequences: the application-facing process also holds the Kubernetes permissions required for agent lifecycle operations. These permissions must be scoped to the resources Cubebot manages. A process failure affects request handling and reconciliation together. Durable intent and restart reconciliation remain necessary; in-memory coordination cannot be the sole record of accepted work.

Scope: this decision selects the backend process structure. [One active orchestrator with automatic restart](#one-active-orchestrator-with-automatic-restart) selects one active instance with automatic restart and accepts recovery downtime. Agents run in their own sandboxes, and PostgreSQL remains the authoritative structured store. [Dedicated TanStack Start frontend with Bun and Vite](console.md#dedicated-tanstack-start-frontend-with-bun-and-vite) adds a dedicated frontend server; backend domain rules and sandbox reconciliation remain in the Rust orchestrator. [PostgreSQL owns agent definitions and lifecycle intent](#postgresql-owns-agent-definitions-and-lifecycle-intent) makes PostgreSQL agent definitions and lifecycle intent authoritative, with no Agent custom resource required initially.

**Rationale.** The chosen deployment keeps installation and operation simple, with one orchestrator artifact and no internal controller network protocol. Module interfaces provide implementation separation without an additional deployed process. Alternative considered: a separate controller would isolate Kubernetes management credentials and allow independent restarts and scaling, at the cost of another deployment and coordination between processes. The user selected the combined process.

**What changed.** The original module list included scheduling (lane admission) and memory persistence with memory policy, and the lifecycle module translated desired state into Kubernetes operations only. Under [Agent-centric execution in a sandbox boundary](sandbox.md#agent-centric-execution-in-a-sandbox-boundary) lane scheduling and memory policy moved into the agent, and under [In-house VMM as the agent sandbox](sandbox.md#in-house-vmm-as-the-agent-sandbox) lifecycle reconciliation targets either Kubernetes or Docker through the `Sandbox` interface.

**Open details.** Recovery-time targets; reconciliation mechanics; Kubernetes workload objects and permissions; internal concurrency; graceful shutdown; deployment and upgrade behavior; and observability.

**Validation scenarios.** Accept work while sandbox creation is delayed; reconcile an agent after process restart; retry failed cluster operations without creating competing executors; keep ordinary requests responsive while reconciliation waits; and restrict lifecycle operations to authorized managed resources.

## One active orchestrator with automatic restart

Accepted 2026-10-23. Amended 2026-10-27: running agents keep working during an outage under their long-lived authority, including starting queued work ([Bounded continuation during disconnection](#bounded-continuation-during-disconnection)).

**Decision.** Run one active orchestrator instance with automatic restart. The user selected this model on 2026-10-23 and accepted recovery downtime for the initial release. Multiple active replicas and standby failover are outside the initial deployment design.

During an orchestrator outage, its client interfaces and agent state operations are unavailable. New work cannot be acknowledged as durably accepted through the unavailable orchestrator. Already accepted work remains in PostgreSQL. Under [Bounded continuation during disconnection](#bounded-continuation-during-disconnection), running agents continue their work within existing authority and limits, with durable local journaling and reconnection reconciliation.

On restart, the orchestrator recovers durable intent, observes existing agent execution, and reconciles pending operations before resuming coordination. It must not assume every previously running turn failed or blindly repeat actions whose outcome is uncertain. Recovery must distinguish accepted work, observed completion, and unresolved outcomes.

One active instance is an operational invariant, not merely a desired replica count. Restart, upgrade, and network-partition handling must prevent an old instance from acting concurrently with its replacement. The mechanism will be specified with execution ownership and recovery; this decision does not require a general multi-replica scheduling implementation.

Kubernetes replacement proves the old pod terminated before taking over. A Docker or database-only instance has no pod to inspect, so it relies on the session advisory lock: holding the lock proves the previous holder's database session ended, its unreleased claim is replaced, and a process still running behind a lost session fails its next ownership check.

Consequences: orchestrator failure and upgrades interrupt coordination and console operations until recovery succeeds. Automatic restart does not guarantee a particular recovery time or success when configuration, PostgreSQL, or cluster capacity is unavailable. PostgreSQL availability and backup policies remain separate decisions.

[Initial qualification at 100 registered and 25 working agents](packaging.md#initial-qualification-at-100-registered-and-25-working-agents) sets the initial qualification target at 100 registered agents and 25 working simultaneously on meaningful goals. This target must be measured with the selected single-active-orchestrator design; it is not a current capacity claim. [Coordinated maintenance upgrades](packaging.md#coordinated-maintenance-upgrades) selects coordinated maintenance upgrades with checkpointing and recovery checks before resuming eligible execution.

**Rationale.** A single active instance keeps deployment and operation simple while retaining durable coordination and explicit restart recovery. Capacity will be measured against representative workloads rather than inferred from replica count. Alternative considered: multiple replicas could serve requests through an instance failure but require coordinated scheduling and lifecycle ownership, stale-owner protection, and connection recovery across replicas. The user chose restart recovery with downtime instead.

**What changed.** The original let running agents continue only their current turns during an outage. Since [Agent-centric execution in a sandbox boundary](sandbox.md#agent-centric-execution-in-a-sandbox-boundary), they may also start queued work they already hold, and stop only for work that needs the orchestrator, revocation or a stopped sandbox.

**Open details.** Recovery-time targets; readiness and health checks; shutdown and upgrade mechanics; enforcement of exclusive active ownership; recovery reconciliation; and PostgreSQL availability.

**Validation scenarios.** Process failure and automatic restart; recovery of accepted messages; reconnection of agents already running; uncertain external-action outcomes; unavailable PostgreSQL during startup; and replacement without competing active orchestrators.

## Bounded continuation during disconnection

Accepted 2026-10-23. Amended 2026-10-27: there is no finite per-turn authorization window; the agent works under its long-lived sandbox authority, may start queued work while disconnected, and stops only for work that needs the orchestrator, revocation or a stopped sandbox ([Agent-centric execution in a sandbox boundary](sandbox.md#agent-centric-execution-in-a-sandbox-boundary)).

**Decision.** Agents may continue work while their orchestrator connection is unavailable. The user selected continuation on 2026-10-23. The agent works under its long-lived sandbox authority, using available context: it may continue current turns and start queued work it already holds. Disconnection limits only what needs the switchboard: it cannot receive new messages or assignments, claim another task, directly deliver a room message, obtain new memories, or obtain anything else from the unavailable orchestrator. It stops when revoked or when its sandbox is stopped.

Disconnection grants no new tool, credential, data, or spending authority. Operations requiring unavailable state or permission changes wait for reconnection. [Autonomous tool use within granted permissions](#autonomous-tool-use-within-granted-permissions) establishes autonomous invocation within existing grants, without a Cubebot per-action approval workflow. Execution budgets still apply.

The agent keeps a durable local journal on persistent storage for unsynchronized execution progress and results. This is recovery evidence, not a second independently mutable copy of the platform database. PostgreSQL remains authoritative for platform state. Local memory changes remain unsynchronized until the orchestrator records them; other lanes or agents must not assume those changes are centrally visible.

Usage: a turn already researching with available context may continue model and permitted tool work during an orchestrator restart. Under [Direct provider inference with propagated authentication](#direct-provider-inference-with-propagated-authentication), model requests go directly to the provider using already-propagated valid authentication. It records progress locally. On completion it retains its result and waits to synchronize.

On reconnection, the agent authenticates, reconciles its execution identity and authority, and submits journaled progress through idempotent orchestrator operations. Central acknowledgement determines when records can be removed from the journal. Known results and uncertain outcomes must be reconciled without blindly repeating tool effects. A rejected or stale execution report does not erase evidence of actions that actually occurred.

When authority is revoked, a budget is exhausted, or durable journaling cannot continue, the engine stops starting further actions and retains recovery state. Already-issued actions may still finish; pausing cannot undo external effects. The design must define handling for those in-flight operations. A restarted agent does not gain permission to resume merely because a journal exists.

Consequences: permission and cancellation changes cannot be assumed immediately visible to a disconnected agent. Revocation means revoking the agent credential or stopping the sandbox, and reassignment must account for an old executor that may still be acting. Local journal durability, capacity, replay, and cleanup become required runtime behavior. Persisted records must distinguish local completion from centrally acknowledged completion.

**Rationale.** Continuation preserves useful progress through coordination outages while limiting work to what the agent can do without current central state. A per-turn window inside the sandbox bought little security, since any credential in the sandbox is reachable by code the agent runs there. Alternative considered: pausing at the next safe point immediately upon disconnection would simplify recovery but interrupt progress during even short outages. The user selected continuation with durable buffering and reconciliation.

**What changed.** The original allowed each active lane to continue only its current turn, accepted no new assignments, started no new queued turns while disconnected, and required continuation to have a finite authorization window that expired. [Agent-centric execution in a sandbox boundary](sandbox.md#agent-centric-execution-in-a-sandbox-boundary) removed the finite per-turn window and the ban on starting queued work; the agent keeps its sandbox credential for the sandbox's lifetime. Open details about authorization duration and renewal and clock and expiry handling no longer apply and were dropped; credential lifetime, rotation and the revocation path while the orchestrator is unavailable are open in that section. The local journal and idempotent reconciliation remain.

**Open details.** Local journal format and limits; action-intent recording; idempotency and reconciliation protocol; spending estimates and budgets; in-flight tool handling; and safe reassignment while an old executor may still act.

**Validation scenarios.** Model and tool work during disconnection; starting already-held queued work during disconnection; turn completion before reconnection; revocation or sandbox stop with a tool in flight; a permission revoked centrally during the outage; journal storage exhaustion; sandbox failure before synchronization; repeated journal submission; and reconnection after the orchestrator has marked execution uncertain.

## Autonomous tool use within granted permissions

Accepted 2026-10-23. Amended 2026-10-27: tools run with the runtime inside one sandbox, and disconnected use follows the sandbox's long-lived authority ([Agent-centric execution in a sandbox boundary](sandbox.md#agent-centric-execution-in-a-sandbox-boundary)).

**Decision.** Agents act autonomously within granted permissions. The user selected this model on 2026-10-23. Once an authorized manager grants a capability, the agent may use it without human approval for each invocation. The initial design does not include a per-action approval queue or agent-specific approval mode.

Usage: an agent granted permission to send email may send email during its assigned work without requesting approval for every message. An agent with read-only access cannot obtain sending permission merely by requesting it in a prompt or choosing another tool. The grant's scope determines which operations and resources are permitted.

Permission checks still apply at execution. Grants do not bypass resource scope, credential validity, execution budgets, cancellation rules, or installation restrictions. The authority to grant capabilities is separate from the capability itself; agents cannot expand their own permissions unless a future explicit decision grants that administrative authority.

Ownership: authorization owns grant semantics and evaluation. Tool execution, in the agent, enforces the resulting constraints, including when tools can invoke other programs or access the network. Prompt instructions alone are insufficient enforcement. The detailed controls for terminals, browsers, and connectors remain part of the tool design.

Consequences: managers must understand the authority a grant conveys, particularly for tools that can modify external systems. Grant descriptions and operational records must make that authority and its use inspectable. Cubebot must report denied or expired access rather than silently elevate it. External systems may still impose their own authentication or approval requirements.

During disconnection, existing grants may be used only within the agent's existing authority and execution limits under [Bounded continuation during disconnection](#bounded-continuation-during-disconnection). A missing grant, revoked authority, or unavailable required state causes the affected operation to wait or fail according to its contract. Reconnection is required to obtain new central authorization. Immediate revocation cannot be assumed visible while disconnected.

Related decisions: terminal, browser and agent-directed programs run inside the agent's own sandbox, whose edge is the trust boundary ([Agent-centric execution in a sandbox boundary](sandbox.md#agent-centric-execution-in-a-sandbox-boundary)). [Agent-directed software installation within grants](sandbox.md#agent-directed-software-installation-within-grants) permits software installation within grants without per-package approval, with enforcement based on actual execution, filesystem, and network authority. [Public internet access with grants for private destinations](sandbox.md#public-internet-access-with-grants-for-private-destinations) grants public internet reachability by default and requires explicit grants for private destinations, subject to stricter installation restrictions; application credentials and operation permissions remain separately enforced. [Native core tools with MCP integrations](#native-core-tools-with-mcp-integrations) keeps core tools native and uses MCP as the default external extension path; MCP tools follow the same grant and autonomous invocation policy, and connecting a server does not authorize every tool it exposes. [Tool allowlists with supported resource restrictions](#tool-allowlists-with-supported-resource-restrictions) selects named tool allowlists and supported resource restrictions, with honest capability reporting and enforcement across available execution paths. [Trigger management within ordinary agent autonomy](#trigger-management-within-ordinary-agent-autonomy) includes managing triggers (routines) for an agent's own work by default, within existing authority and installation limits, without a separate grant.

**Rationale.** This model supports unattended agent work and places the human decision at capability assignment rather than every tool invocation. Alternatives considered: mandatory review for designated sensitive actions would interrupt those workflows for human approval. Configurable approval policies per agent would support supervised and autonomous modes but add policy and approval lifecycle complexity. The user selected autonomous operation within grants.

**What changed.** The original placed agent-directed terminal and browser workloads in a tool container separate from the credential-holding runtime within the same pod, and bounded disconnected use by a finite offline authorization window. [Agent-centric execution in a sandbox boundary](sandbox.md#agent-centric-execution-in-a-sandbox-boundary) replaced both: runtime and tools share one sandbox, and the agent holds long-lived sandbox authority. The orchestrator no longer authorizes individual tool calls.

**Open details.** Grant representation; manager delegation; integration-specific resource checks and credential scopes; terminal and browser enforcement; budget accounting; denial behavior; revocation propagation; and operational records.

**Validation scenarios.** Execute a granted external write without a human approval step; deny an ungranted operation; prevent a tool from escaping its enforced scope through another execution path; reject revoked authority; and apply the disconnection policy without creating a new approval workflow.

## Agent creation as a granted capability

Accepted 2026-10-23.

**Decision.** Agents may create other agents when explicitly granted that capability. The user selected this model on 2026-10-23. The creating agent requests creation through the orchestrator; it receives no direct Kubernetes or platform database authority. Permission to collaborate under [Explicit agent messaging and delegation](rooms.md#explicit-agent-messaging-and-delegation) does not itself grant permission to create agents.

The creation grant defines the permitted scope of provisioning and what authority the creator may assign to a new agent. Creation remains subject to installation quotas, resource capacity, and budget rules. Creating agents must not reset or bypass applicable aggregate limits.

New agents belong to the installation under [Agent ownership](identity.md#agent-ownership), with managers assigned through the authorized provisioning policy. The creator's identity is recorded as provenance, not as a separate ownership category. A new agent receives its own identity and memory ownership. Creation does not implicitly copy the creator's memories, workspace, credentials, or room memberships.

The creator may assign only permissions allowed by its creation grant. In particular, the ability to create an agent does not automatically authorize granting that new agent further creation rights, installation administration, arbitrary credentials, or access to additional rooms. These choices must be explicit in the grant and validated by the orchestrator.

Usage: an agent authorized to assemble a team requests an additional specialist with a purpose, configuration, and permitted resources. The orchestrator validates the request, records the new identity and provisioning intent, and manages execution under the existing lifecycle policy. The caller can then delegate work through the ordinary task interfaces when the new agent is eligible.

Ownership: authorization validates creation and delegated permissions. Agent management owns identity and manager assignment. Lifecycle modules own admission and sandbox provisioning. Retried creation requests must resolve to the same accepted creation rather than unintentionally provisioning duplicate agents.

Consequences: provisioning, attribution, quota enforcement, and eventual retirement become part of agent-driven workflows. Creation is a centrally authorized operation and cannot occur while disconnected under [Bounded continuation during disconnection](#bounded-continuation-during-disconnection). Automatic approval of a valid grant does not guarantee immediate compute availability.

[Persistent and task-bound created agents](#persistent-and-task-bound-created-agents) permits persistent or task-bound lifetime according to the creation grant, defaulting agent-initiated creation to task-bound workers. [Fresh memory with an explicit briefing](memory.md#fresh-memory-with-an-explicit-briefing) initializes new agents with fresh memory and an explicit briefing, without copying the creator's saved memory.

**Rationale.** Separately granted creation supports dynamically assembled teams while retaining centralized control of resources and authority. Agents without that capability can still collaborate with existing peers. Alternative considered: restricting creation to humans would make provisioning predictable but prevent authorized workflows from independently adding specialists or workers. The user selected agent-initiated creation within grants.

**Open details.** Detailed quota and budget accounting; manager assignment defaults; creation interface; model and resource configuration limits; room admission; creation depth; quota reservations; and cleanup after partial provisioning or parent cancellation.

**Validation scenarios.** Authorized creation succeeds; an agent without the grant is denied; excessive delegated permissions are rejected; duplicate submission does not duplicate identity; concurrent creation respects quotas; sandbox provisioning failure preserves an inspectable state; and creation during disconnection does not allocate an unauthorized agent.

## Persistent and task-bound created agents

Accepted 2026-10-23.

**Decision.** Support persistent and task-bound lifetimes, selected at creation within the creation grant. The user selected this model on 2026-10-23. Task-bound workers are the default for agent-initiated creation; persistent creation requires the grant to permit it. Human-initiated creation defaults are not changed by this decision.

Persistent agents remain available for future work until explicitly retired by an authorized actor. Task-bound agents are associated with an originating task and are automatically retired once that task and its dependent work are settled. Retirement makes an agent ineligible for execution. It does not itself delete memories, conversation history, results, or workspace files.

Lifetime and idle policy are separate. Both kinds use [Agent idle lifecycle](#agent-idle-lifecycle): sleeping agents remain eligible to wake, while retired agents do not. An always-running idle policy cannot extend a task-bound lifetime after retirement. Both kinds remain installation-owned agents with their own identities and memories, using the same engine and permission model.

Usage: a lead creates a persistent specialist for recurring analysis and several task-bound researchers for one investigation, if its grant permits both choices. The researchers can sleep between assignments while their originating work remains active. Once the investigation and its dependent work are settled, they retire and their records remain subject to retention policy. The specialist remains eligible for later assignments.

The orchestrator validates the lifetime against the creation grant and stores the originating task for a task-bound agent. It owns automatic retirement. Retirement must account for active execution, pending results, and disconnected work; a terminal parent record alone is not permission to discard unresolved child execution or recovery evidence.

Consequences: the task system must define when originating and dependent work is settled, including failure and cancellation. The console must distinguish a sleeping worker from a retired one. Retirement can release execution resources while retained records and workspace data continue consuming storage. [Retain durable data until explicitly deleted](memory.md#retain-durable-data-until-explicitly-deleted) retains durable records and workspaces until explicit deletion, with no automatic age-based expiry by default. [Cancel selected work and its descendants](rooms.md#cancel-selected-work-and-its-descendants) applies these retirement rules once originating and dependent work are settled after cancellation, preserving history and recovery evidence.

**Rationale.** One agent model with an explicit lifetime policy supports lasting teams and temporary swarms without making every created worker permanently available. Alternatives considered: making every created agent persistent would simplify lifetime behavior but require explicit cleanup of temporary workers. Making every created agent task-bound would prevent creation of reusable specialists without another lifecycle transition. The user selected both lifetimes within grants.

**Open details.** Failure and cancellation settlement; assignments outside the originating task; promotion to persistent lifetime; explicit retirement authority; reactivation; descendant lifetimes; and retention and cleanup mechanics. No automatic promotion or memory transfer is implied.

**Validation scenarios.** A permitted persistent creation succeeds; a task-only grant rejects persistent creation; omitted lifetime defaults to task-bound for agent creation; a sleeping worker wakes while its task remains active; unsettled dependent execution prevents premature cleanup; retirement stops future scheduling; and retained results remain accessible after retirement.

## Default agent model with optional purpose overrides

Accepted 2026-10-23.

**Decision.** Each agent has one default model and optional model overrides for memory extraction, dreaming, and context compaction. An unspecified purpose inherits the default. The user selected this model on 2026-10-23. Authorized managers configure these choices from the installation-managed connections the agent may use under [Installation-managed provider connections](identity.md#installation-managed-provider-connections) and [Installation-wide connection access by default](identity.md#installation-wide-connection-access-by-default).

Usage: a manager selects the agent's default model and starts using it without configuring maintenance separately. Later, the manager can assign a different extraction model while leaving dreaming and compaction on the default. Main-lane and task-lane work use the default; the agent does not autonomously select a different model from a list.

A model selection identifies both a provider connection and a model on that connection. An override may use another eligible connection. Choosing an override does not grant new connection access or bypass budgets, provider limits, or data-access restrictions. The console should distinguish inherited settings from explicit overrides and show the effective selection for each purpose.

Ownership: agent configuration owns the default and optional overrides. The engine resolves the effective model from the inference purpose through a small shared interface. Authorization validates use of the selected connection. Provider adapters translate requests and expose relevant capabilities; each consuming module defines its requirements. Memory and context modules should not independently invent selection rules.

Implementation direction: represent the choice as a connection reference plus model identifier, and resolve it from agent configuration and inference purpose. Record the effective selection with the execution or maintenance record so users can inspect which model actually performed the work.

An absent override inherits the default. An explicitly configured but unavailable, unauthorized, or incompatible override is an execution problem, not an absent override. [Retry and wait for the selected model](#retry-and-wait-for-the-selected-model) selects bounded retries and visible waiting, with no automatic fallback to the default or another connection or model.

Purpose compatibility includes the input and output needed for the operation. For compaction, the summarizing model must be able to process the chosen material, and its output plus retained exchanges must fit the continuing work model's context budget. A cheaper maintenance model is an option, not an assumption of equal quality. Extraction, dreaming, and compaction require purpose-specific evaluation.

Consequences: initial configuration requires only the default. Overrides can change cost, latency, output quality, and the provider receiving the operation's context. Configuration changes need explicit activation semantics so running work does not observe an unexplained model switch.

Scope: this decision covers the agent's generative inference purposes. It does not select embedding models or the room coordinator's model, add per-task overrides, choose concrete providers, or define automatic fallback.

**Rationale.** Deterministic configuration with inheritance keeps setup to one choice while allowing deliberate maintenance tuning. Alternatives considered: one model for all agent inference is simpler but prevents maintenance-specific tuning. Autonomous selection from an approved list adds inference-driven routing and another behavior to constrain and evaluate. The user selected deterministic configuration with inheritance and optional overrides.

**Open details.** Exact types; capability validation; supported model catalog and model-catalog discovery; configuration-change activation and configuration-version handling; provider-specific parameters; usage accounting; concrete model qualification; embedding selection; coordinator configuration; and detailed retry and recovery mechanics under [Retry and wait for the selected model](#retry-and-wait-for-the-selected-model).

**Validation scenarios.** All purposes inherit a sole default; an extraction override leaves other purposes unchanged; removing an override restores inheritance; an ungranted connection is rejected; an unavailable explicit override is not silently replaced; compaction respects both models' budgets; and execution records identify the model actually used.

## Retry and wait for the selected model

Accepted 2026-10-23.

**Decision.** Retry eligible transient failures within limits, then leave affected work visibly blocked pending recovery or an authorized configuration change. The user selected this behavior on 2026-10-23. Cubebot does not automatically switch models, providers, or connections, and does not offer automatic fallback chains.

The selected connection and model remain in effect for the operation. An unavailable explicit purpose override does not fall back to the agent's default. A manager may deliberately change configuration, but activation and safe continuation rules must be defined separately; a configuration edit must not silently restart completed work.

Usage: a working agent encounters a temporary provider failure. The runtime retries the model request within its retry and execution budgets. If access remains unavailable, the console shows the affected work as waiting for that model, with its reason and last attempt. Recorded task progress and completed tool actions remain intact. When recovery permits another attempt, execution resumes from recorded state rather than replaying the whole task.

Ownership: provider adapters classify failures and expose retry information where available. The execution module owns bounded request retries and partial-response handling. Durable waiting and subsequent admission are owned by the agent's lane scheduling and recorded in the ledger. Authorization and budgets continue to constrain every attempt; provider recovery does not restore revoked authority or reset spending limits.

Implementation requirements: use bounded backoff for retryable failures and avoid synchronized retries across agents sharing a connection. Authentication failures, revoked grants, exhausted budgets, incompatible capabilities, and invalid requests must not enter an indefinite transient-error retry loop. Their recorded reason identifies the required intervention or recovery condition.

Waiting is a durable execution state, not an unbounded busy loop. The console distinguishes retrying, waiting on provider recovery, and conditions requiring intervention, without exposing secrets. Users can still inspect and cancel affected work. Detailed lane reservation, sleep eligibility, recovery probing, notification, and deadline behavior remain separate scheduling decisions. A waiting lane must not be treated as safely idle solely because no inference call is in flight.

Interrupted responses require explicit reconciliation. A retry may incur provider cost again and does not imply exactly-once inference. The engine must not execute incomplete tool calls or repeat already-executed tools merely because the response stream or a later inference request failed. Uncertain action outcomes retain their recovery state. Disconnected retries remain subject to the authority and journaling rules of [Bounded continuation during disconnection](#bounded-continuation-during-disconnection).

This policy also applies to agent maintenance purposes. Failed compaction retains the last valid context under [Rolling context compaction](memory.md#rolling-context-compaction); if ordinary work cannot fit without compaction, it waits visibly. Maintenance failure must not falsely report successful extraction, dreaming changes, or context replacement. Provider waiting remains distinct from settled task failure, budget pauses and cancellation ([Contain delegated failures and let the parent adapt](rooms.md#contain-delegated-failures-and-let-the-parent-adapt)).

Consequences: model outages or prolonged rate limits can delay autonomous work. No alternate provider is contacted automatically. Recovery scheduling, wait visibility, and preservation of partial progress become essential for unattended operation.

**Rationale.** Predictable model use preserves deliberate model selection, provider exposure and cost. Alternative considered: an explicitly configured fallback chain could preserve progress through some outages but would change model behavior, provider exposure, and possibly cost. The user selected predictable model use with retry and waiting.

**What changed.** The original assigned durable waiting and admission to an orchestrator scheduling module and bounded disconnected retries by a finite offline authority window. Under [Agent-centric execution in a sandbox boundary](sandbox.md#agent-centric-execution-in-a-sandbox-boundary) lanes are scheduled inside the agent loop and the agent holds long-lived sandbox authority; retries remain bounded by retry and execution budgets.

**Open details.** Exact retry classification, timing, and attempt limits; backoff and jitter; shared-connection recovery coordination; partial-stream reconciliation; wait deadlines and notifications; lane and sandbox lifecycle while waiting; configuration-change activation; and coordinator or embedding failure policies.

**Validation scenarios.** A transient failure succeeds on a bounded retry; repeated failures enter visible waiting; an unavailable override never invokes the default; authentication failure avoids a retry storm; cancellation stops further attempts; restart preserves waiting state; recovery does not replay a completed tool; and retry budgets remain bounded during orchestrator disconnection.

## Direct provider inference with propagated authentication

Accepted 2026-10-23. Amended 2026-10-27: provider authentication is propagated for the sandbox's lifetime, not as per-turn inference grants, and runtime and tools share one sandbox ([Agent-centric execution in a sandbox boundary](sandbox.md#agent-centric-execution-in-a-sandbox-boundary)).

**Decision.** Agent runtimes call model providers directly. The orchestrator propagates the authentication material and configuration needed for the connections each agent is eligible for, for the lifetime of its sandbox. The user selected direct calls on 2026-10-23 and explicitly specified that agents have access to the same authentication mechanism as the orchestrator through this propagation.

This covers the supported authentication mechanism for the selected provider connection, rather than requiring every agent to be configured or logged in independently. It does not distribute all installation credentials to every agent. [Installation-wide connection access by default](identity.md#installation-wide-connection-access-by-default) allows connection eligibility through an all-agents baseline or specific grants; propagation still supplies only the applicable material for connections that agent is eligible for. Provider authentication remains distinct from the agent's identity when calling Cubebot, Kubernetes authority, and database access; agents still receive no platform database credentials under [Agent state access through the orchestrator](#agent-state-access-through-the-orchestrator).

Usage: an administrator configures a provider connection and makes it available to an agent. The orchestrator authenticates that agent and supplies the applicable provider configuration and authentication material. The agent runtime constructs model requests, authenticates directly with the provider, consumes the response stream, and reports execution and usage records to the orchestrator. Model payloads and streams do not require an orchestrator inference proxy.

Ownership: the orchestrator owns connection administration, eligibility evaluation, and propagation of authorized authentication state. Provider adapters in agent runtimes implement the same supported authentication semantics used for orchestrator inference. The agent engine owns its local model and tool loop. Shared adapter code should keep those semantics consistent without introducing a second agent engine or separately administered personal connections.

Propagation must use an authenticated, confidential channel and identify the connection and authentication-state revision. Updates must not accidentally restore an older credential revision. Reconnecting agents reconcile their state with the orchestrator.

Sharing an authentication mechanism does not imply shared refresh authority. [Orchestrator-owned provider authentication refresh](identity.md#orchestrator-owned-provider-authentication-refresh) makes the orchestrator the sole refresh owner and authoritative distributor of updated authentication state. Agent runtimes do not refresh independently. Subscription support still requires qualification for this runtime and deployment; propagation alone does not establish provider permission or compatibility.

Trusted runtime access to provider authentication material is required by this decision. It does not imply placing secrets in model context, ordinary logs, shared workspace files, or the environment of agent-executed commands. The runtime and its tools share one sandbox whose edge is the trust boundary; any credential inside the sandbox is reachable by code the agent runs there, and isolation between the runtime and its own tools is not a security boundary.

During disconnection, an agent may continue using already-propagated, valid authentication within its sandbox authority and budgets. Missing or unusable authentication cannot be obtained from an unavailable orchestrator. No new grant is created by a locally cached credential. Provider failure follows [Retry and wait for the selected model](#retry-and-wait-for-the-selected-model) without automatic connection or model switching.

Consequences: the orchestrator does not carry agent inference streams. Provider credentials exist in more execution environments and are held for longer, so isolation, rotation, and revocation need explicit design and must actually work. Locally enforced authorization is not the same as invalidating a copied external credential. A compromised sandbox can spend that agent's provider budget until its credential is revoked. Shared-account limits and aggregate spending still require coordination even though calls are distributed.

Scope: this decision governs agent inference and provider authentication. It does not grant arbitrary tools access to provider secrets, define tool-integration credential distribution, or move the room coordinator out of the orchestrator. [Encrypted credentials with a separate installation key](identity.md#encrypted-credentials-with-a-separate-installation-key) stores authoritative credentials encrypted in PostgreSQL with a separately held installation key; that key is not propagated to agents. [Shared goal budgets with optional cost limits](#shared-goal-budgets-with-optional-cost-limits) selects aggregate goal budgets for delegated work, with monetary limits optional and unset by default; enabled limits require usage reconciliation despite direct provider calls.

**Rationale.** Direct calls keep each new model request independent of orchestrator availability. Alternative considered: proxy all inference through the orchestrator, retaining provider credentials there. That would centralize request enforcement but make each new model request depend on orchestrator availability. The user selected direct inference and propagated authentication.

**What changed.** The original propagated authentication as per-turn, versioned inference grants within finite execution authority, and relied on separate runtime and tool containers in the agent pod to keep provider credentials away from agent-executed programs. [Agent-centric execution in a sandbox boundary](sandbox.md#agent-centric-execution-in-a-sandbox-boundary) replaced that with propagation for the sandbox's lifetime and one sandbox holding runtime and tools, whose edge is the trust boundary. Direct inference and orchestrator-owned refresh remain. The open detail "isolation enforcement under separate containers" became sandbox isolation, tracked in that section.

**Open details.** Exact delivery mechanism and local secret storage, persistence and encryption; bootstrap identity; update acknowledgement protocol; refresh coordination and recovery under [Orchestrator-owned provider authentication refresh](identity.md#orchestrator-owned-provider-authentication-refresh); credential rotation and revocation; distributed quota and budget enforcement; authentication-state versioning; and provider qualification.

**Validation scenarios.** An eligible agent calls its provider directly; an ineligible connection is not propagated; an authentication update replaces the correct connection revision; reconnect rejects stale state; existing work continues during a short orchestrator outage within limits; expired authentication causes visible waiting; provider secrets stay out of model and tool outputs; and aggregate usage remains attributable across agents.

## Initial OpenAI, Anthropic, and compatible-endpoint support

Accepted 2026-10-24. Amended 2026-11-04: OpenRouter joins as a named provider with its own adapter on Chat Completions, listing every model it returns, including those that cannot call tools, which the console marks. Agents still think with one connection; a provider that stops being it keeps its encrypted key (`provider_saved`, migrations/0047) until someone forgets it, so switching back needs no pasting. Amended 2026-11-04: at the user's request Google Gemini and xAI Grok join as named providers, which replaces this section's statement that native Gemini is out of scope. Both adapters use Chat Completions, sharing OpenRouter's wire code (`modules/providers/src/completions.rs`). Gemini lists through its native API, the only one reporting context windows, narrows tool schemas to the subset it accepts and replays thought signatures. Grok's Chat Completions is labelled legacy by xAI with no shutoff date; move it to Responses when one is announced.

**Decision.** Target native OpenAI and Anthropic provider adapters, plus an adapter for configurable OpenAI-compatible endpoints, in the first release. The user selected this scope on 2026-10-24. Native Google Gemini support is outside the initial adapter scope. Cubebot retains its own Rust agent loop under [Native Rust agent engine](#native-rust-agent-engine).

Usage: an administrator configures a supported provider connection and makes it available to selected agents. A manager chooses an available model through that connection. For a compatible hosted or local endpoint, the administrator configures its endpoint and supported authentication without assuming that every capability of the native OpenAI adapter is available.

Ownership: provider adapters translate inference requests, streams, tool calls, usage, errors, and capability differences. The runtime owns context, tool execution, and turn progression. Installation administration owns connections and agent-use grants. Agents call providers directly under [Direct provider inference with propagated authentication](#direct-provider-inference-with-propagated-authentication); the orchestrator owns refresh under [Orchestrator-owned provider authentication refresh](identity.md#orchestrator-owned-provider-authentication-refresh).

API credential access and subscription-based authentication are separate qualification paths. ChatGPT and SuperGrok subscriptions are supported as personal connections under [Personal subscription connections](identity.md#personal-subscription-connections). Claude subscription connections remain a desired integration from the product direction. Their actual availability for the native Rust engine, account eligibility, authentication lifecycle, supported inference path, and deployment compatibility must be researched and demonstrated before support is claimed. This scope decision neither guarantees those paths nor replaces them silently with supervising external agent engines.

OpenAI-compatible means support for a documented and tested protocol subset, not universal compatibility with every endpoint using that description. Specify the supported request APIs and capability matrix during design and implementation. Validate streaming, tool calling, tool-result ordering, context limits, image inputs where needed, authentication, cancellation, and usage reporting for qualified endpoints. Unsupported required capabilities must produce a clear configuration or execution error rather than silently changing the requested behavior.

Endpoint configuration is installation-managed. Credentials must remain scoped to their configured destination, including during redirects or endpoint changes. A private local-model endpoint needs the appropriate deployment network path; configuring inference does not grant arbitrary tool programs access to that endpoint or its credentials. Exact validation and network enforcement remain design work.

Provider selection does not add automatic fallback. [Retry and wait for the selected model](#retry-and-wait-for-the-selected-model) retains the selected model and connection through bounded retry and visible waiting. Missing usage information must not be reported as zero, particularly when an optional cost limit is enabled. Connection-wide limits still apply across agents sharing an account.

This decision selects inference adapter scope. Exact models, SDK or HTTP implementation, protocol versions, embeddings for hybrid retrieval, and the room coordinator's configured model remain engineering proposals or qualification details. The specification must identify these explicitly rather than infer that every inference model provides every supporting capability.

**Rationale.** Concentrating on OpenAI, Anthropic and compatible endpoints focuses initial implementation and qualification while retaining multi-provider access. Alternatives considered: include a native Gemini adapter in the initial release as well. The user selected OpenAI, Anthropic, and compatible endpoints.

**Open details.** Adapter interfaces and concrete protocols; capability discovery and validation; initial tested models and compatible endpoints; subscription authentication feasibility; error classification; usage accounting; embeddings; and provider-specific recovery behavior.

**Validation scenarios.** Complete a tool-using turn through each native adapter and a qualified compatible endpoint; stream output and recover partial-stream failure; reject unsupported required capabilities; enforce connection grants; rotate credentials without disclosure; preserve selected-model waiting; and qualify each claimed subscription path separately. Accepting this decision executed no provider or subscription integration.

## Native core tools with MCP integrations

Accepted 2026-10-23. Amended 2026-10-27: local MCP servers run inside the agent's sandbox rather than a separate tool container ([Agent-centric execution in a sandbox boundary](sandbox.md#agent-centric-execution-in-a-sandbox-boundary)).

**Decision.** Keep memory, rooms, messaging, delegation, terminal, and browser tools native to Cubebot. Use Model Context Protocol, MCP, as the default extension path for external integrations, adding native adapters when concrete requirements justify them. The user selected this model on 2026-10-23. Cubebot acts as an MCP client for this integration path; exposing Cubebot itself as an MCP server is not selected here.

Usage: an installation administrator configures an external tool server under [Administrator-managed integration configuration](identity.md#administrator-managed-integration-configuration), and an authorized administrator or delegated manager assigns approved tools to an agent. The agent can invoke those tools alongside native tools through Cubebot's execution interface. For example, it can retrieve a project record through an integration, work with local files, and report a result to a room without the external server owning the room or task lifecycle.

Ownership: native domain modules own Cubebot's core operations. An MCP adapter, in the agent, owns protocol negotiation, discovery, request and result translation, and connection errors. Authorization owns which external tools may be exposed and invoked. The execution module owns invocation identity, budgets, cancellation, and outcome records across native and MCP tools. This does not require a new deployed gateway or an in-process plugin loader.

Connecting a server does not grant all of its tools. Tool identity must include its configured server identity so an external tool cannot impersonate a native tool or another integration. Discovered schemas and capability changes must be reconciled with grants. Newly discovered tools do not receive authority merely because the server was already connected. [Tool allowlists with supported resource restrictions](#tool-allowlists-with-supported-resource-restrictions) combines named tool allowlists with supported resource restrictions and external credential scopes.

MCP metadata describes a tool; it is not proof of its behavior or a replacement for credential scopes and enforcement. Validate arguments and bound outputs at the interface. Treat returned content and tool descriptions as external data, without allowing them to rewrite Cubebot's permissions or runtime instructions. Network policy under [Public internet access with grants for private destinations](sandbox.md#public-internet-access-with-grants-for-private-destinations) still applies to the integration path; routing through a trusted process must not become a bypass.

Within granted permissions, MCP tool use follows [Autonomous tool use within granted permissions](#autonomous-tool-use-within-granted-permissions) without a new per-call approval queue. External authentication or interactive requirements must be surfaced when actually required. Configuring an MCP connection does not automatically authorize additional protocol features such as server-requested model inference or user interaction. Supported features must be explicitly selected and qualified.

Disconnects, timeouts, and cancellation do not prove that a remote operation had no effect. Recovery must preserve uncertain outcomes and use a server's supported reconciliation or idempotency mechanisms where available rather than blindly retrying external writes. Successful protocol delivery and successful application work remain distinct outcomes.

Integration definitions are administrator-managed under [Administrator-managed integration configuration](identity.md#administrator-managed-integration-configuration). [Installation-managed external account connections](identity.md#installation-managed-external-account-connections) makes external account connections installation-managed as well. [Local and remote MCP server support](#local-and-remote-mcp-server-support) selects both remote Streamable HTTP endpoints and local stdio subprocesses; local third-party servers run inside the agent's sandbox. [Explicit tool search with on-demand discovery](#explicit-tool-search-with-on-demand-discovery) selects a small core tool set with explicit tool search and on-demand definition loading; catalog discovery does not expand grants.

Consequences: Cubebot gains a standard extension interface while retaining responsibility for permission enforcement, observability, and failure handling. Server quality and capability vary and need qualification. MCP support alone does not provide a turnkey connector catalog, inbound event system, or universal authentication flow.

Protocol reference reviewed during design: [MCP tools specification](https://modelcontextprotocol.io/specification/2025-11-25/server/tools).

**Rationale.** Native core tools keep reliable control over Cubebot's own behavior; MCP avoids making every external service depend on Cubebot development and maintenance. Alternative considered: native integrations only initially. That would give direct control over each adapter but require Cubebot-specific development for every service. The user selected native core tools and MCP extensibility, with targeted native adapters still possible.

**What changed.** The original ran local third-party servers in the tool container, separate from the credential-holding runtime. [Agent-centric execution in a sandbox boundary](sandbox.md#agent-centric-execution-in-a-sandbox-boundary) runs local MCP servers in the agent's one sandbox with the runtime; the agent adapts MCP itself, so client placement is no longer open.

**Open details.** Exact grant representation and handling of schema changes; supported protocol versions and transport mechanics; local-server lifecycle; credential storage and lifecycle; grant vocabulary; catalog freshness; supported optional protocol features; and integration qualification.

**Validation scenarios.** Expose and invoke a granted external tool alongside native tools; deny an ungranted or newly added tool; distinguish identical tool names from different servers; handle incompatible schemas and bounded outputs; report authentication failure; cancel without falsely claiming rollback; and retain an uncertain external-write outcome after connection loss.

## Local and remote MCP server support

Accepted 2026-10-23. Amended 2026-10-27: local servers run as subprocesses inside the agent's sandbox, with no separate tool container ([Agent-centric execution in a sandbox boundary](sandbox.md#agent-centric-execution-in-a-sandbox-boundary)). Amended 2026-11-03 by the user: servers are installation configuration that administrators manage in the console, not deployment settings; each is available to every agent by default or to agents granted it, under [Installation-wide connection access by default](identity.md#installation-wide-connection-access-by-default), and an agent's Connections tab switches a limited server for that agent. Environment and header values are stored encrypted and delivered with each turn.

**Decision.** Support both remote MCP endpoints using Streamable HTTP and local MCP server subprocesses using stdio. The user selected both modes on 2026-10-23. Both feed the same tool-execution contract and follow the same authorization, resource accounting, and outcome-recording rules.

Usage: an agent uses a remote organizational integration for project records and a local MCP program for workspace processing. The runtime invokes either through the granted tool interface without requiring the model to manage protocol sessions or subprocess pipes.

Local third-party servers execute as subprocesses inside the agent's sandbox, not inside the runtime process. They are not given the runtime's model-provider credentials, platform identity, or database access through their environment, arguments or results; the sandbox edge, not the process, is the trust boundary. Local deployment introduces no extra pod or sandbox per MCP server. Installation of required executables and dependencies follows [Agent-directed software installation within grants](sandbox.md#agent-directed-software-installation-within-grants).

Ownership: the MCP adapter owns protocol lifecycle and request/result translation for both transports. Tool execution owns local subprocess supervision, cancellation, and output bounds. Connection configuration describes the authorized endpoint or local executable and necessary configuration.

A local process definition must identify its executable, arguments, working directory, and approved environment or credential references. Starting a process is code execution under the tool environment's grants. Registering an MCP tool does not authorize unrelated commands or new credentials. Permission to install software does not by itself authorize registering new tools or expanding an agent's grants.

Remote endpoints can be hosted outside the installation or elsewhere in the cluster. Cubebot connects to them; this decision does not make Cubebot responsible for deploying every remote server. Private endpoint access still requires the applicable grant under [Public internet access with grants for private destinations](sandbox.md#public-internet-access-with-grants-for-private-destinations). Remote network reachability and MCP authentication do not replace application-level authorization.

Transport state is distinct from Cubebot agent, lane, and task identity. A restarted local server or reconnected remote session must be associated with the correct execution records and revalidated tool catalog. Neither transport reset proves an earlier operation had no effect. Do not replay an uncertain external write merely to recreate a protocol session.

Local process counts, startup concurrency, and resource use must remain bounded. Unused servers need lifecycle handling consistent with agent sleep, while active or uncertain invocations must be reconciled before shutdown. Local-server logs and protocol output must remain distinguishable and must not expose secrets through ordinary agent results.

Consequences: Cubebot must qualify two transport paths and supervise local server execution. Local packages need not be written in Rust; Cubebot's engine and integration implementation remain Rust, while third-party tools may require other runtimes. Neither transport implies support for every optional MCP feature or legacy transport.

Configuration authority: [Administrator-managed integration configuration](identity.md#administrator-managed-integration-configuration) reserves integration registration and configuration for administrators; managers can assign approved integrations only within delegated authority. [Installation-managed external account connections](identity.md#installation-managed-external-account-connections) selects installation-managed external account connections only.

Protocol reference reviewed during design: [MCP transport specification](https://modelcontextprotocol.io/specification/2025-11-25/basic/transports).

**Rationale.** Both modes support the selected workspace and organizational integration use cases. Alternatives considered: remote-only support would avoid local process supervision but require every integration to have a server deployment. Local-only support would place all integration dependencies and processes in agent environments.

**What changed.** The original ran local servers in the agent's tool container, separated from the trusted runtime container, and left MCP client placement and forwarding across the runtime/tool interface open. [Agent-centric execution in a sandbox boundary](sandbox.md#agent-centric-execution-in-a-sandbox-boundary) put runtime, tools and local MCP servers in one sandbox, so there is no runtime/tool interface to forward across and the client lives in the agent; that open detail was settled.

**Open details.** Authentication and secret storage; supported protocol versions and optional features; local process sharing across lanes, startup strategy, restart policy and cleanup timing; executable packaging; remote session recovery; and per-integration qualification.

**Validation scenarios.** Discover and invoke granted tools through each transport; run a local server without runtime credentials in its environment; deny unauthorized registration or private endpoint access; distinguish protocol output from logs; handle local process exit and remote disconnect; preserve uncertain invocation outcomes; and release idle local processes without losing active work.

## Tool allowlists with supported resource restrictions

Accepted 2026-10-23.

**Decision.** Combine named tool allowlists with Cubebot-enforced resource restrictions where an integration supports them, alongside external credential scopes. The user selected this model on 2026-10-23. Each integration must expose which restrictions Cubebot can actually enforce. Unsupported restrictions require an adequately scoped external credential or configuration rejection; they must not be represented as enforced prompt instructions.

Usage: an administrator grants an agent an issue-reading tool on a selected account connection, restricted to two repositories. Cubebot checks the intended resource before dispatch. A request for a third repository is denied even if the connected account could otherwise access it. If the integration cannot enforce repository selection, that narrow policy requires external credentials or server configuration that demonstrably limit access to those repositories.

Ownership: authorization owns grant evaluation and the intersection of applicable restrictions. An integration-specific adapter or qualified policy mapping identifies resources and implements supported checks. Tool execution invokes those checks before dispatch and records the applied policy and outcome. The generic MCP adapter does not infer authorization semantics from parameter names or model judgments.

Grant identity includes the agent or applicable execution scope, registered integration, selected account connection, permitted tools, and supported resource restrictions. [Installation-wide connection access by default](identity.md#installation-wide-connection-access-by-default) adds the connection's all-agents baseline as a source of use authority. Its approved tool and resource scope applies without separate per-agent assignment; specific-grants mode requires explicit grants within that scope. Managers may assign only within their delegated authority under [Administrator-managed integration configuration](identity.md#administrator-managed-integration-configuration). A resource restriction cannot authorize an unlisted tool, another account, a private network destination, or a capability missing from the external credential.

Restriction support must describe its limits. Checks must account for relevant aliases, default resources, indirect identifiers, bulk operations, and caller-supplied URLs before claiming enforcement. Read operations that can return multiple resources need an appropriately scoped query or upstream enforcement; silently discarding unauthorized results after broad retrieval does not establish that the data was never accessed. Exact resource identifiers, schema, and qualification cases remain integration-specific.

An MCP tool description or read-only annotation is not proof of a restriction. A schema change that invalidates a policy mapping must not silently broaden access. Unknown or unresolved resource identity must not pass a restrictive grant by default. Behavior for changed capabilities and configuration revisions must be explicit and tested.

The restriction must hold across the execution paths available to the agent. A check on an MCP call is insufficient if the agent can use the same broad credential through an unrestricted terminal program, browser session, or alternate endpoint. Qualification must account for credential placement and reachable bypass paths, remembering that any credential inside the agent's sandbox is reachable by code the agent runs there ([Agent-centric execution in a sandbox boundary](sandbox.md#agent-centric-execution-in-a-sandbox-boundary)). Use narrower upstream credentials, an enforcing execution path, or reject a claimed scope that the deployment cannot uphold. This decision does not assume that a generic proxy can enforce every external service's semantics.

Within a valid grant, execution remains autonomous under [Autonomous tool use within granted permissions](#autonomous-tool-use-within-granted-permissions). There is no per-call approval queue. During orchestrator disconnection, resource checks may use only valid existing authorization and sufficient trusted state within the limits of [Bounded continuation during disconnection](#bounded-continuation-during-disconnection); missing required state does not permit a broader operation.

Consequences: fine-grained restrictions require integration-specific support and validation. Arbitrary MCP servers can still be used within their honestly represented authority, but cannot claim unsupported resource controls. The console must distinguish Cubebot-enforced restrictions, external restrictions, and unavailable controls.

**Rationale.** Additional Cubebot restrictions add narrower policy when credentials are broader, with upstream scopes as a complementary control. Alternative considered: allowlist named tools and rely entirely on external credential scopes for resource limits. That is simpler but cannot add narrower Cubebot policy when credentials are broader.

**Open details.** Grant schema; resource identity and normalization; supported integration mappings; capability discovery and validation; delegation representation; policy activation and revocation; credential placement; bypass-resistant execution paths; and denial diagnostics.

**Validation scenarios.** Permit an allowed tool and resource; deny an unlisted tool or resource; reject an unsupported narrow policy; prevent account substitution; handle aliases and implicit defaults; reject stale schema mappings; verify bulk-read scope; and demonstrate that alternate tool paths cannot reuse broader credentials to bypass a claimed restriction.

## Explicit tool search with on-demand discovery

Accepted 2026-10-23.

**Decision.** Expose a small core tool set and provide explicit tool search that remains available throughout execution. Agents search the eligible catalog and load relevant tool definitions on demand. The user selected this model and explicitly required tool search analogous to memory search on 2026-10-23.

Usage: an agent needs to find open issues for a project. It searches for that capability, receives bounded results identifying eligible tools and their configured connections, and selects the relevant tool. Cubebot makes its current definition available for invocation. The agent can refine its search when results are ambiguous or report that no eligible capability exists.

The analogy to [memory search](memory.md#tool-initiated-memory-retrieval) is explicit retrieval when needed. The tool catalog is configuration and capability metadata, not the agent's saved knowledge pool. This decision does not select the same index, embedding model, or ranking algorithm for both searches.

Ownership: the tool module owns catalog lookup and definition resolution across native tools and registered MCP integrations. Authorization determines eligibility and checks each invocation. The runtime controls which resolved definitions enter the execution lane's model context. MCP adapters obtain server capabilities under [Native core tools with MCP integrations](#native-core-tools-with-mcp-integrations); an agent's catalog search does not itself register servers or create connections.

Search returns only tools eligible under the requesting agent's effective access, including the all-agents connection baseline or specific grants under [Installation-wide connection access by default](identity.md#installation-wide-connection-access-by-default). Results must distinguish native tools, integrations, and account connections, including tools with identical names. Compact results provide enough description and scope information to choose a capability; resolving a tool supplies its current argument schema and relevant restrictions. Discovering or loading a definition never expands permissions, and a newly advertised server tool does not inherit approval merely through discovery. Broad connection availability does not load every schema into the model context.

Execution rechecks current authority and validates arguments against the applicable definition. Revoked grants, changed schemas, unavailable servers, or missing authentication produce explicit outcomes rather than silent account substitution or broader access. Descriptions and search results remain untrusted metadata and cannot change runtime instructions. If the catalog cannot be reached during disconnection, the runtime reports that limitation; previously resolved tools remain subject to the existing-authority limits of [Bounded continuation during disconnection](#bounded-continuation-during-disconnection).

Loaded definitions are bounded context material that can be released and resolved again. They are not automatically written into long-term memory. Tool search itself must remain accessible so an agent cannot lose the means to rediscover capabilities after compaction.

**Rationale.** On-demand discovery adds a lookup step while keeping ordinary requests smaller as the installation's integrations grow. Alternative considered: include all granted tool schemas upfront. That simplifies initial exposure but grows context with the installation's integrations.

**Open details.** Exact core set; search interface and ranking; catalog freshness; definition-loading mechanics for each provider; per-lane definition budgets and eviction; and discovery diagnostics.

**Validation scenarios.** Find and use an eligible tool absent from initial context; refine an ambiguous search; distinguish identical tool names and accounts; exclude ungranted tools; reject revoked authority at invocation; handle schema changes; report no match or unavailable discovery; and rediscover a released definition after compaction.

## Scheduled and event-triggered work

Accepted 2026-10-23. Amended 2026-11-01: [Routines](#routines) settles the open mechanics with one shape.

**Decision.** Support both schedules and incoming external events as triggers for agent work. The user selected both on 2026-10-23. Trigger occurrences create durable work through the orchestrator and use the existing assignment, admission, and execution machinery. The concrete form is the [routine](#routines): each occurrence is a turn in the agent's permanent conversation, and the supported event source is a webhook.

Usage: a scheduled routine asks an agent to review selected repositories every weekday morning. An authenticated monitoring event asks an agent to investigate an alert. Either can request a wake for a sleeping agent, subject to permissions, budgets, capacity, and lifecycle policy. Triggering work does not guarantee immediate execution.

Ownership: the orchestrator owns trigger configuration, schedule evaluation, event admission, and durable occurrence records. Resulting work enters the agent's conversation and lanes like any message. Event adapters translate supported external inputs into this common path. Agents do not need an always-running sandbox solely to receive events or track schedules. This decision does not introduce a separate workflow engine, message broker, or Kubernetes CronJob for each routine.

A trigger identifies its intended work and destination; a routine belongs to one agent. Each accepted occurrence must retain enough origin and correlation information to explain why work started and relate it to its assignment and outcome (a routine turn carries the routine's id and its origin). The console must expose configured triggers and occurrence status, including queued work, failures, and rejected or skipped occurrences where recorded.

Incoming events are external data. Admission must validate the configured source and payload before creating work. Event content cannot grant tools, select unauthorized accounts, or expand an agent's permissions. Delivery through an approved integration does not make instructions inside the payload trusted system instructions; a webhook body is quoted into the message as a JSON string marked as untrusted data. MCP tool support alone does not imply inbound event support; each supported event source needs an explicit ingress path.

Schedule occurrences and repeated event deliveries need stable identities and deduplication appropriate to their sources. Recovery must account for an occurrence already admitted before a restart. This is not a guarantee of exactly-once external side effects. Per-trigger occurrence handling is described in [Configurable occurrence handling per trigger](#configurable-occurrence-handling-per-trigger).

[Trigger management within ordinary agent autonomy](#trigger-management-within-ordinary-agent-autonomy) includes managing triggers for an agent's own work in ordinary autonomy, within existing scope and installation limits. This does not authorize registering integrations or exposing arbitrary endpoints; external subscription changes require applicable integration permissions. Retired agents cannot be revived by triggers; unavailable targets and admission failures need visible outcomes.

**Rationale.** Recurring routines and reactive work are both part of the intended product; they share durable work admission while retaining source-specific validation. Alternative considered: introduce schedules or external events first and defer the other.

**What changed.** [Routines](#routines) settled these open details: schedule syntax and time zones (five-field cron in UTC); missed-occurrence policy (an overdue routine runs once and is not replayed); grouping mechanics (coalescing); event authentication (a per-routine bearer secret), replay protection (an `Idempotency-Key` seen in the last day) and payload limits (8 KiB of UTF-8); supported sources (webhooks); trigger destinations (the routine's one agent, as a turn in its permanent conversation); rate limits (60 deliveries a minute per routine, then 429); trigger edit scope (an agent changes only routines it manages); and cancellation (deleting a routine cancels its queued and waiting turns and keeps the history). Room destinations are not part of routines.

**Open details.** Time zones other than UTC; event sources other than webhooks; payloads beyond the message, such as files; per-installation limits and backpressure; and occurrence retention.

**Validation scenarios.** Admit scheduled and event-triggered work; wake a sleeping agent; preserve accepted work across orchestrator restart; deduplicate a repeated identifiable occurrence; reject an unauthenticated event; bound a burst; report an unavailable target; and prevent event content from expanding authority.

## Trigger management within ordinary agent autonomy

Accepted 2026-10-23. Amended 2026-11-01: [Routines](#routines) settles the management interface, ownership, quotas and stop semantics.

**Decision.** Include management of triggers for an agent's own work in ordinary agent autonomy. The user selected this model on 2026-10-23. Agents may create and manage their schedules and event triggers by default within installation limits and existing authority, without a separate capability grant or per-change human approval.

Usage: while investigating an issue, an agent schedules a later check or arranges recurring reviews. It can also enable a trigger using an already approved event source it is authorized to use. The orchestrator validates and persists the trigger, then admits future occurrences through the durable work path of [Scheduled and event-triggered work](#scheduled-and-event-triggered-work).

Ownership: the orchestrator owns trigger records and scheduling. Authorization enforces applicable scope and installation restrictions. The agent chooses the trigger's purpose and configuration through native tools: `routine_create`, `routine_list`, `routine_update`, `routine_delete` and `routine_run`. People manage routines with `conversation.send` on the agent and read them with `conversation.read`, so administrators and authorized managers inspect and manage these triggers through their existing scoped authority.

This default covers an agent's own work. The agent may change only routines it manages; a person's edit makes a routine theirs. It does not confer authority to modify another agent's triggers, alter a human-managed trigger, or schedule work in an unauthorized room. Cross-agent delegation continues to use its existing authorization rules. Trigger records must identify their creator, target, configuration revisions, and current status so human intervention is understandable; updates carry the expected revision.

Installation limits apply to active triggers, occurrence rates, queued work, and resulting execution costs. An agent holds at most 32 routines. Creating several triggers cannot bypass aggregate limits. A trigger stores work intent, not a permanent snapshot of permissions: future occurrences and their execution remain subject to current authority and lifecycle rules. Retired agents do not wake because an old trigger remains configured.

Event triggers use administrator-configured sources and installation-managed connections under [Administrator-managed integration configuration](identity.md#administrator-managed-integration-configuration) and [Installation-managed external account connections](identity.md#installation-managed-external-account-connections). Ordinary trigger autonomy does not authorize registration of a new integration, credential acquisition, or arbitrary endpoint exposure: agents cannot create webhook secrets, so their autonomy never exposes an endpoint. A Cubebot event filter and an external service subscription are distinct operations; changing the latter requires the applicable existing integration permissions.

Management operations must reach the orchestrator to become durable. An agent disconnected under [Bounded continuation during disconnection](#bounded-continuation-during-disconnection) cannot independently activate a new central schedule or event subscription. Retrying a creation request after an uncertain response must not create duplicate triggers; a tool call's id makes a retried create or run idempotent.

Human cancellation or disabling must be respected. An agent must not silently undo an explicit human stop through routine reconciliation; because a person's edit makes a routine theirs, the agent cannot undo a person's stop. A disabled routine runs only by hand; its schedule and webhook stop. Deleting a routine cancels its queued and waiting turns and keeps the history.

[Configurable occurrence handling per trigger](#configurable-occurrence-handling-per-trigger) lets each trigger coalesce or not; autonomous trigger management can select this within installation limits.

**Rationale.** Agents doing continuing work need follow-ups and event reactions without an opt-in grant; existing scope checks, visible records, and installation limits remain the controls. Alternatives considered: human-only management, or agent management enabled by a separate explicit grant. Both restrict autonomous follow-up more than the selected default.

**What changed.** [Routines](#routines) settled these open details: trigger ownership and edit scope (an agent changes only routines it manages; a person's edit makes it theirs); management interface (the `routine_*` tools and `conversation.send`/`conversation.read`); default per-agent quota (32 routines); pause and cancellation semantics and handling of pending work (disable stops schedule and webhook; delete cancels queued and waiting turns and keeps history); frequency limits (60 webhook deliveries a minute per routine; schedules at cron minute resolution); and backlog overflow (coalescing keeps at most one running and one pending).

**Open details.** Per-installation limits.

**Validation scenarios.** Create a self-directed trigger without an extra grant; update or remove it within scope; enforce aggregate quotas; deny unauthorized targets and integration registration; respect a human stop; recheck authority for later work; and deduplicate creation after a lost response.

## Configurable occurrence handling per trigger

Accepted 2026-10-23. Amended 2026-11-01: [Routines](#routines) settles the mechanics as coalescing by default, and does not build combined work with references to each occurrence.

**Decision.** Each trigger selects whether its occurrences coalesce. The user selected this configurability on 2026-10-23. Both modes remain subject to installation limits, existing execution admission, and current permissions.

Without coalescing (individual processing), each accepted distinct occurrence is its own turn. For example, each incoming support request requires its own work. Queue limits still apply; this mode does not promise unlimited acceptance or immediate execution.

Coalescing is the default. While one of the routine's turns is queued or waiting, a new occurrence makes no turn, so at most one runs and one is pending. For example, repeated service-health signals produce one follow-up investigation rather than one turn per signal. Coalescing does not cancel or rewrite already-running work. A coalesced occurrence leaves no record.

Ownership: the orchestrator's routine handling owns occurrence identity and the relationship between occurrences and turns. The turn goes through the agent's ordinary lanes. This policy does not change the agent's concurrency limits or create another execution lane.

Admission must handle arrivals racing with turn startup or completion without creating competing pending turns. The implementation must preserve this across orchestrator restarts.

Model context remains bounded. Storage retention and payload-size limits remain separate policies; coalescing does not make event storage unbounded.

Deduplication is separate from coalescing. Redelivery of the same identifiable occurrence (a webhook `Idempotency-Key` seen in the last day) returns the first answer without a second turn, in either mode. Coalescing applies to distinct occurrences of the same routine, not arbitrary unrelated work or different routines.

The configured mode must be visible to humans and agents managing the trigger under [Trigger management within ordinary agent autonomy](#trigger-management-within-ordinary-agent-autonomy). Overflow, rejection, and other admission failures must have explicit outcomes (a webhook over 60 deliveries a minute is 429). Individual mode must not silently switch to coalescing when its queue fills. A routine overdue after downtime runs once and its next time is computed from now, so missed schedule occurrences are not replayed.

**Rationale.** Always processing individually can accumulate redundant work; always combining removes necessary per-occurrence boundaries. Per-trigger selection supports both workflows without requiring a general workflow engine. Coalescing without a ledger keeps routine occurrences as ordinary chat turns instead of a second work queue.

**What changed.** The original "combined processing" accumulated occurrences into one pending assignment with references to each contributing event, kept contributing occurrences inspectable, and forbade silently replacing prior event information with only the latest payload; it also gave the combined set a clear boundary when the assignment started. [Routines](#routines) replaced this with coalescing: a coalesced occurrence leaves no record, and combined work with references to each occurrence is not built. It also settled the default mode (coalesce), overflow handling (429 above 60 webhook deliveries a minute), and missed schedule occurrences during downtime (run once, not replayed).

**Open details.** Per-installation queue limits; batching latency; mode changes with pending work; and interaction with failures and retries.

**Validation scenarios.** Create separate turns without coalescing; keep one pending turn when coalescing; retain one follow-up during active work; deduplicate redelivery; recover after restart; handle startup and arrival races; and report capacity exhaustion without silently changing modes. Inspecting contributing events of a coalesced turn is not supported.

## Routines

Accepted 2026-11-01.

**Decision.** A routine is a stored prompt, a shell command, or both, for one agent. The user selected this on 2026-11-01. It settles the mechanics of [Scheduled and event-triggered work](#scheduled-and-event-triggered-work), [Trigger management within ordinary agent autonomy](#trigger-management-within-ordinary-agent-autonomy) and [Configurable occurrence handling per trigger](#configurable-occurrence-handling-per-trigger) with one shape.

- **Occurrences.** A routine runs on demand by a person or by the agent, on a schedule, or by webhook. Each occurrence is a turn in the agent's permanent conversation with the routine's id and its origin (`person`, `agent`, `schedule` or `webhook`), inserted and woken like any chat message, so [Agent execution lanes](#agent-execution-lanes), retries and cancellation apply unchanged. The turn acts as the person who ran it, or, for the other origins, the person who created the routine or whose turn the agent created it in.
- **Coalesce.** A routine coalesces by default: while one of its turns is queued or waiting, a new occurrence makes no turn, so at most one runs and one is pending. Without coalescing every occurrence is its own turn. A coalesced occurrence leaves no record; combined work with references to each occurrence is not built.
- **Schedules.** Five-field cron in UTC (minute, hour, day of month, month, day of week with 0 or 7 as Sunday; `*`, lists, ranges and steps; when both day fields are restricted either matches). A tick every 30 seconds claims due routines with `FOR UPDATE SKIP LOCKED`. A routine overdue after downtime runs once and its next time is computed from now, so missed occurrences are not replayed.
- **Webhooks.** `POST /api/hooks/{routine}` with a bearer secret a person creates or rotates in the console, shown once and stored as a SHA-256 digest; a wrong or missing secret is 401. The body, at most 8 KiB of UTF-8, is quoted into the message as a JSON string marked as untrusted data. An `Idempotency-Key` header seen in the last day returns the first answer without a second turn. Each routine accepts 60 deliveries a minute; more are 429. Agents cannot create webhook secrets, so agent trigger autonomy never exposes an endpoint.
- **Commands.** The turn delivery carries the command, which the agent runs as the tool user with bash before the model, journaled like a tool call; the model sees its result as untrusted program output. Without a prompt the turn answers with the result and makes no model call or provider request. A rerun after an interruption never runs the command again.
- **Management.** People manage routines with `conversation.send` on the agent and read them with `conversation.read`. The agent's `routine_create`, `routine_list`, `routine_update`, `routine_delete` and `routine_run` tools manage its own routines; it may change only routines it manages, and a person's edit makes a routine theirs, so the agent cannot undo a person's stop. A tool call's id makes a retried create or run idempotent. An agent holds at most 32 routines. Updates carry the expected revision. A disabled routine runs only by hand; its schedule and webhook stop. Deleting a routine cancels its queued and waiting turns and keeps the history.

**Rationale.** Making every occurrence an ordinary turn reuses the conversation, lanes, retries and cancellation instead of building a second work queue. Alternatives considered: a trigger table per source with an occurrence ledger, which records coalesced and rejected occurrences but adds a second work queue beside chat turns. Kubernetes CronJobs, which [Scheduled and event-triggered work](#scheduled-and-event-triggered-work) rules out. Replaying every missed schedule after downtime, which floods an agent with stale work.

**Open details.** Time zones other than UTC; payloads beyond the message, such as files; per-installation limits; occurrence retention; event sources other than webhooks.

**Validation scenarios.** The next time of a schedule; a coalescing routine keeps one pending turn; a wrong secret is 401; a replayed idempotency key makes no second turn; a command-only routine completes without a provider request.

## Shared goal budgets with optional cost limits

Accepted 2026-10-23. Amended 2026-10-27: budget limits are checked from ledger usage reports, not enforced per call ([Agent-centric execution in a sandbox boundary](sandbox.md#agent-centric-execution-in-a-sandbox-boundary)).

**Decision.** A goal and its delegated work share one aggregate execution budget. Child tasks and agents may have tighter limits, but delegation cannot multiply or reset the goal's configured allowance. The user selected this model on 2026-10-23 and emphasized that monetary cost is not a primary concern: cost limits are optional configuration, unset by default.

A goal here means the root work request and the tasks and continuations performing that request. Users can start work without entering a spending allowance, and Cubebot does not invent a hidden monetary ceiling. Existing installation, provider, resource, concurrency, and authorization constraints continue to apply. An unset monetary limit does not remove the authority limits on disconnected execution under [Bounded continuation during disconnection](#bounded-continuation-during-disconnection).

Usage: a research goal delegates collection, analysis, and document production to several agents. If the goal has a configured cost or other supported aggregate limit, all attributable child work consumes it. If no cost limit is configured, work proceeds without a monetary admission gate while still respecting permissions and available execution capacity.

Ownership: the task module preserves root-goal attribution through delegation, transfer, retries, and continuation. An execution-budget module within the orchestrator owns configured limits, reservations, and reconciled usage, and checks limits from the usage agents report to the ledger rather than per provider call. Runtime and tool adapters report actual or estimated consumption with identifiable operations. Cost observations are distinct from enforced limits, and unknown usage must not be presented as zero.

For enabled aggregate limits, allocate bounded allowances before distributed work starts and reconcile consumption afterward. Concurrent allocations must fit within the remaining allowance. A child allocation comes from the parent's remaining allowance; it is not an extra allowance. With monetary limits disabled, no artificial cost allocation is needed solely to authorize work.

Agents call providers directly under [Direct provider inference with propagated authentication](#direct-provider-inference-with-propagated-authentication). Accounting must distinguish reservations, reported usage, estimates, and unresolved calls. Lost responses and delayed usage must not cause duplicate charging in Cubebot's ledger or premature reuse of an outstanding reservation. A runtime restart or task transfer does not reset consumption. Billing accuracy and enforceable cost bounds depend on provider capabilities; the product must state estimation limits rather than promise an exact external invoice ceiling. A compromised sandbox can spend that agent's provider budget until its credential is revoked.

Disconnected work respects existing authority and any already allocated bounded allowance for enabled limits. It cannot obtain more allowance from an unavailable orchestrator. Reassignment must account for an old executor that may still act; expiration alone does not prove an in-flight request consumed nothing.

Agents cannot evade a configured root limit by creating another agent, issuing a direct request, or scheduling a continuation of the same goal. Distinguishing a continuation from an independently authorized new goal must be explicit. Recurring routines and background maintenance need their own attribution rules; this decision does not charge unrelated work to whichever goal last used the agent. Delegation under [Explicit agent messaging and delegation](rooms.md#explicit-agent-messaging-and-delegation) does not create additional allowance, and recovery after a delegated failure retains the same goal ancestry and budgets ([Contain delegated failures and let the parent adapt](rooms.md#contain-delegated-failures-and-let-the-parent-adapt)).

Limit changes require applicable management authority and a visible record. Autonomous delegation does not confer authority to increase its own allowance. [Preserve progress and pause on configured budget exhaustion](#preserve-progress-and-pause-on-configured-budget-exhaustion) selects a resumable pause with preserved progress when a configured limit is exhausted. No default token, wall-time, or monetary values are selected here.

**Rationale.** The shared model makes a configured goal limit meaningful across collaboration. Alternatives considered: independent allowances for each delegated task, bounded only by broader installation limits. Mandatory cost budgeting was explicitly excluded by the user's clarification.

**What changed.** The original left room for enforcing allowances per distributed call. [Agent-centric execution in a sandbox boundary](sandbox.md#agent-centric-execution-in-a-sandbox-boundary) checks budget limits from ledger usage reports instead, and notes that a compromised sandbox can spend its agent's provider budget until revoked.

**Open details.** Exact domain naming; supported dimensions and user-facing controls; configuration and inheritance; attribution for routines and maintenance; exact accounting, reservation sizes and reservation mechanics; provider estimation; unresolved usage reconciliation; limit-management authority; and pause and resume mechanics.

**Validation scenarios.** Start work without a cost limit; share an enabled limit across parallel delegates; apply a tighter child limit; preserve consumption through transfer and retry; reconcile duplicate reports; avoid recycling uncertain reservations; and prevent child creation or scheduled continuation from resetting a configured allowance.

## Preserve progress and pause on configured budget exhaustion

Accepted 2026-10-23.

**Decision.** Pause affected work with preserved progress when a configured execution budget is exhausted. The user selected this model on 2026-10-23. Record the reason and notify the responsible authorized user. An authorized limit change can make the work eligible to continue. Monetary limits remain optional and unset by default under [Shared goal budgets with optional cost limits](#shared-goal-budgets-with-optional-cost-limits).

Usage: a goal reaches an enabled aggregate limit while several specialists are working. Cubebot stops admitting further affected actions, retains their results and continuation state, and shows the goal as paused for that limit. After an authorized user supplies sufficient allowance and requests or permits continuation under the eventual resume contract, work continues from reconciled state rather than starting over.

Ownership: the execution-budget module detects exhausted allowances and stops new allocations. Affected starts are blocked. Runtime and tool modules settle or reconcile in-flight activity and preserve durable progress. The task module owns the resumable state and dependency relationships. Notifications follow the recipient permissions and user preferences of [In-app notifications with optional push](console.md#in-app-notifications-with-optional-push).

Exhaustion is scoped to the limit reached. A root-goal limit affects the root and all attributable delegated work. A tighter child-only limit pauses that child and prevents dependent progress where its result is required; it does not exhaust the parent's remaining allowance or block unrelated goals. An installation-wide exhausted limit can affect multiple goals.

A pause stops new affected actions; it cannot undo already-issued requests or external effects. Distinguish a requested pause, draining or uncertain execution, and confirmed paused state. Preserve completed outputs, pending tool outcomes, and reported or reserved usage. The system must not issue an extra unbudgeted model call merely to summarize why it paused. A deterministic status can explain the limit and link to existing progress.

When agents are disconnected, their existing allocations and authority still bound execution. Central exhaustion cannot be assumed instantly visible to them. Accounting must include outstanding allocations and uncertain consumption before allowing further work elsewhere. Restart or transfer cannot clear the pause, reset consumption, or blindly replay an action.

An authorized limit adjustment is necessary when the applicable allowance is insufficient, but it is not enough to bypass other conditions. Resume rechecks permissions, cancellation state, dependencies, provider access, and unresolved execution outcomes. It retains the same goal identity and prior consumption. A separately cancelled goal must not restart because its limit was later increased; cancellation is distinct from this pause ([Cancel selected work and its descendants](rooms.md#cancel-selected-work-and-its-descendants)).

Preserved progress means durable task state, recorded outputs, context checkpoints, workspace files, and recoverable action evidence. It does not promise suspension of arbitrary processes or live browser sessions. Once execution is safely settled, ordinary idle lifecycle rules may allow the agent to sleep. Unrelated authorized work may still run within its applicable capacity and limits.

**Rationale.** A resumable pause better supports persistent work while making the configured limit effective. Alternative considered: end the goal with partial results and require a new request.

**Open details.** Exact supported dimensions; limit-state schema; pause acknowledgement and draining; notification aggregation; manual versus automatic eligibility after adjustment; idle lifecycle integration; and recovery of individual tool types.

**Validation scenarios.** Pause a goal and delegates at an enabled aggregate limit; pause only a child at its tighter limit; retain partial results across restart; reconcile an in-flight side effect; respect offline allocations; resume without duplicate work after adjustment; preserve cancellation; and never pause for an unset monetary limit.

## Agent usage and monthly budgets

Accepted 2026-11-04.

**Decision.** Record what every model response used, per agent, and let a person cap what one agent may spend in a calendar month. The user asked on 2026-11-04 for visibility and some level of governance over agents' spending. This is the first concrete limit under [Shared goal budgets with optional cost limits](#shared-goal-budgets-with-optional-cost-limits) and follows [Preserve progress and pause on configured budget exhaustion](#preserve-progress-and-pause-on-configured-budget-exhaustion); it limits an agent, not a goal, and goal budgets stay unbuilt.

- **Ledger.** `usage_events` holds one row per response a provider reported: a turn's, a dream's, or the room coordinator's (no agent). Tokens are what the provider said; usage it did not report stays unrecorded and is never guessed.
- **Cost.** A provider's own figure wins, which today is OpenRouter's and Grok's. Otherwise cost is an estimate from the model's price as that provider publishes it. The user decided on 2026-11-04 that prices come from each provider, not from one aggregator. OpenAI and Anthropic have no price API and their Models APIs carry no price, so their adapters read the Markdown their pricing pages serve; OpenRouter's public model listing has prices of its own. The orchestrator asks only providers with a saved connection, at most daily, sending no key. A page it cannot read leaves the last good prices in place and logs a warning. Offline or for an unlisted model the cost stays NULL, the console says so, and the row is priced if a price arrives later. Standard-tier short-context rates apply; long-context, batch and regional rates and Anthropic cache writes are not modelled, so an estimate can run under the bill.
- **Budget.** `agents.monthly_budget_usd`, unset by default, is set with `agent.manage` and recorded in the audit log. Past it, a new turn waits as `budget_exhausted` without calling the model, a running turn stops at its next response through the ordinary cancellation channel, and dreams are not admitted. A command-only routine still runs: it calls no model. An unpriced response cannot exhaust a budget.
- **Resuming.** Raising or removing the budget, or a new month that leaves room, requeues the paused turns, except those with tool effects of unknown outcome, which stay waiting for a person as any interrupted turn does.
- **Visibility.** People read an agent's usage with `conversation.read`. The Situation room shows each person the agents they may read; only administrators see the coordinator's spend, the provider and installation totals.

**Rationale.** A shipped price table goes stale silently and would be wrong for any new model; a provider's own page is current and an absent price is visible. Reading a documentation page is more fragile than an API, so a page that yields nothing is an error rather than an empty price list. Stopping a running turn at its next response bounds overspend to the responses already in flight, which an already-issued request can always exceed.

**Open details.** Budgets for an installation, team or goal; alerts outside the console; retention of usage rows; a person overriding a price; switching to a price API if a provider offers one.
