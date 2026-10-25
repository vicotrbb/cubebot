# Cubebot software specification

Date: 2026-10-24, updated 2026-10-25 for ADR 0081. Status: engineering proposal implementing the accepted ADR baseline, ready for design review. The [architecture](../../ARCHITECTURE.md) defines the deployment and ownership map. This document proposes concrete contracts; illustrative identifiers and signatures are not implemented endpoints or migration files.

## 1. Caller-facing contracts

The ordinary caller should express intent once and inspect an operation if the response is lost:

~~~text
receipt = work.send_message(actor, conversation, content, request_id)
operation = work.get_operation(actor, receipt.operation_id)
goal = work.get_goal(actor, receipt.goal_id)
work.steer(actor, assignment_id, instruction, expected_revision, request_id)
work.cancel(actor, target_goal_or_task, expected_revision, request_id)
work.accept_submission(actor, goal_id, submission_id, expected_revision, request_id)
~~~

Sending acknowledges durable receipt, not successful routing or model execution. A receipt may not yet contain a goal when coordinator routing is pending. All mutating commands carry an actor, a unique request identity, and any necessary expected resource revision. The server derives installation and effective permissions from authenticated context.

Idempotency is scoped to actor, command kind, and request identity. Retain a digest of the relevant request: repeating the same command returns its outcome; reusing the identity for different content returns a conflict. Authorize before disclosing even a cached operation result. Keep durable command identities with their records so a reconnect cannot create another message after a short cache expires.

Errors distinguish invalid input, permission denial, revision conflict, capacity exhaustion, maintenance, unsupported capability, transient dependency failure, and outcome requiring reconciliation. A timeout is not evidence that a mutation failed.

## 2. Domain identities and types

Use distinct Rust newtypes for IDs, not interchangeable strings throughout domain code. Parse wire values at ingress. The following is a conceptual schema:

| Type | Essential fields and invariants |
| --- | --- |
| Installation | ID, policy revision, maintenance state; one installation per database configuration |
| HumanIdentity | ID, local or issuer/subject login binding, active status; email is not an identity key for automatic linking |
| Team / Grant | Local membership; fixed role or named capability, resource scope, delegable subset |
| Agent | ID, definition revision, managers, instructions revision, provider selection, lifetime, idle policy, resources |
| RuntimeIncarnation | Agent ID, pod UID, epoch, authenticated connection, validity and observed status |
| Conversation / Message | Agent-direct, room, or task conversation; immutable message identity, actor, sequence, content references |
| Room | Members, instructions revision, routing configuration; membership is not unrestricted content access |
| Goal | Root intent, responsible owner, requirements revision, acceptance policy, optional budget limits |
| Task | Goal ID, optional parent task, owner, conversation reference, dependency outcomes, status and revision |
| Lane / Turn | Agent main or task lane; one active turn per lane, execution epoch, context checkpoint |
| Invocation | Stable operation ID, tool/schema revision, grant revision, intent digest, effect classification, outcome |
| Submission | Goal and requirement revision, immutable deliverable references, check results, limitations, author |
| MemoryRevision | Agent owner, prior revision, content, status, provenance, extraction/dream/correction origin |
| DreamRun | Agent, admission/checkpoint state, source range, committed change references and summary |
| Skill / SkillRevision / Install | Shared library identity; immutable package; each agent pins its own selected revision |
| Connection | Installation owner, provider/integration type, endpoint, optional secret reference, auth revision, enabled state, access mode, policy revision, approved scope |
| Trigger / Occurrence | Target and grants, schedule/event definition, grouping policy, deduplication identity |
| Upload / Blob / ArtifactReference | Transfer identity; immutable bytes; domain-authorized publication reference |
| MaintenanceRun | Operation type, barriers, checkpoints, capture or upgrade progress, recovery evidence |

Room assignments, delegated tasks, and trigger work all use Task. A Goal is the accounting and cancellation root. Multiple explicit mentions produce separate root assignments by default unless an explicit existing-goal continuation is selected; do not invent an automatic lead or a shared budget across independent user requests.

All derived execution preserves root attribution. A recurring trigger starts an independent goal per accepted occurrence or combined group unless it explicitly continues existing work. An agent cannot convert goal-owned work into an unrelated trigger to escape cancellation or limits.

Agent-initiated creation requires an explicit creation grant covering quantity, resource profiles, permitted capabilities, and allowed lifetime. The request selects persistent or task-bound lifetime within that grant; omitted lifetime defaults to task-bound for agent-created workers. The child receives fresh memory and an explicit source-attributed briefing. It does not inherit the creator's memory, workspace, rooms, credentials, or installed skills automatically. Any supplied configuration and skill selection must be independently permitted. Human-created lifetime defaults remain a UI proposal.

The child's connection eligibility is evaluated from current installation policy, including all-agents connections under ADR 0081. That baseline is independent of the creator's specific grants and does not copy the creator's credential material.

Sleep is the default idle policy; managers may choose always-running agents. Only settled idle execution can sleep. Messages, triggers, and scheduled dreaming can wake eligible agents. Retired task-bound agents do not wake, and their retained data remains inspectable through authorized platform operations.

## 3. State machines

### Agent state

Separate lifecycle intent from observation:

- Lifetime: persistent, or task-bound with an originating task.
- Desired availability: eligible, suspended, or retired. Idle policy determines whether eligible agents keep a pod.
- Observed execution: absent, provisioning, ready, working, dreaming, draining, unavailable.

An agent can have several working lanes, some blocked and some progressing. Derive the overview from those records and preserve the detailed states. Dreaming requires no active or uncertain lane and no higher-priority eligible work. Retirement disables future admission; cleanup waits for settled dependent work.

### Work and review

Proposed execution states are queued, running, waiting, paused, settling, and terminal. Waiting has a typed reason such as children, provider, human input, or capacity. Paused records a reason and resumption requirements. Settling records unresolved tool or executor outcomes; it is never silently treated as idle.

Goal result states are open, awaiting_acceptance, completed, failed, and cancelled. Execution status and result status are separate so review can consume no agent capacity.

~~~mermaid
stateDiagram-v2
    [*] --> Open
    Open --> AwaitingAcceptance: valid submission / review required
    Open --> Completed: valid submission / agent completion
    AwaitingAcceptance --> Completed: authorized acceptance of current revision
    AwaitingAcceptance --> Open: changes requested
    Open --> Failed: unable to meet requirements / settled work
    Open --> Cancelling: stop accepted
    AwaitingAcceptance --> Cancelling: stop accepted
    Cancelling --> Cancelled: affected execution settled
~~~

Cancelling is the visible aggregate of cancellation intent plus settling execution. Store a durable cancellation version immediately; do not wait for every descendant update before blocking admission.

Task outcomes distinguish success, failure, and cancellation, with partial artifacts and unresolved-effect references. Unknown external effects stay in settling until reconciled or explicitly recorded for operator intervention. A failed child wakes eligible parent handling; it does not mark all siblings failed. Proposed failed-parent behavior: drain or cancel unfinished owned descendants before settling the parent, retaining completed outcomes. This prevents abandoned work without introducing goal-wide failure on every child error.

### Completion and human acceptance

Proposed acceptance policy is agent_completion or human_acceptance with a designated authorized reviewer set. Default to agent_completion. For human-created review-required goals, propose the initiating user as reviewer when authorized; otherwise require an explicit eligible reviewer. Scheduled review-required work must name a reviewer or review team at configuration time.

A submission includes requirement-by-requirement disposition, evidence references, checks actually performed, outputs, and limitations. The orchestrator checks structural obligations and configured automated checks; the agent assesses semantic adequacy. Never infer all requirements are met from the existence of a final response.

Reject completion while goal-owned execution remains unsettled or required children remain unresolved. Optional branches must be explicitly settled and accounted for. A configured budget pause cannot be bypassed by an extra model call to produce a completion report.

Human acceptance references a submission ID and current goal/requirements revision. Changes requested create a continuation under the same goal and budget, preserving the rejected submission. A reviewer can accept or request changes; review does not confer access to credentials or hidden sources. Absent review does not auto-accept. Once execution starts, the responsible agent cannot weaken review policy. Propose policy changes only through explicit authorized goal management and invalidate incompatible submissions.

Cancellation and acceptance serialize against the same goal revision. The losing operation receives a conflict. A user who wants additional work after a completed or cancelled goal creates an explicit linked follow-up, rather than mutating terminal history into a different outcome.

## 4. Work transactions and scheduling

Use PostgreSQL tables for pending work, lane ownership, and delivery/outbox state. Claim bounded batches under short transactions using row locks where needed; never hold a transaction during model, browser, or Kubernetes calls. Application workers remain asynchronous tasks in the single orchestrator process.

Proposed scheduling order is maintenance/authority checks, ancestor cancellation, goal limits, lane eligibility, provider/account capacity, then execution admission. Eligible queued human messages take precedence under ADR 0005, followed by queue age. Expose starvation metrics; do not silently replace that accepted priority rule with a different policy.

Main-lane context includes only its selected assignment and relevant conversation. Task lanes use task context. Default task concurrency is two, configurable one through four; main concurrency is one. Installation capacity and shared provider constraints may reduce actual admission. Start with a proposed installation lane cap of 50 for the qualification profile, independent of the 25-working-agent target, and tune using evidence.

Each claim records task, lane, epoch, authority expiry, and checkpoint. A unique active-lane constraint prevents duplicate ownership in PostgreSQL. Context/result commits require the current epoch and expected revision.

Propose a 120-second maximum disconnected authority window with renewal while connected and a conservative local monotonic deadline. This is an engineering starting value, not an accepted product limit. A restarted runtime reauthenticates before new actions; it cannot treat a stale saved expiry as new authority. A tool already running at expiry is cancelled or settled through its tool-specific contract; an external side effect may remain uncertain.

The orchestrator uses a dedicated PostgreSQL session lock for active ownership plus a monotonic owner epoch in durable operations. Losing the database connection disables new control actions immediately. In-flight Kubernetes requests and partitioned old processes still require reconciliation: the lock alone is insufficient fencing. Hold replacement work when the old writer could still act. The qualification plan must establish how confirmed process termination or node fencing closes that uncertainty. Availability may be sacrificed to preserve single ownership.

## 5. Message, room, and delegation flows

### Human message

1. Validate the actor and conversation access, persist message and routing work, return receipt.
2. For direct agent chat, enqueue its main lane. For room mentions, validate every target and create independent assignments. Report invalid targets explicitly.
3. With no room mention, invoke the configured installation coordinator model with bounded room context and eligible members.
4. Validate its structured selection against current membership and task revisions. Persist the routing result and work atomically. A stale result is retried or reported, never broadcast to every member.
5. Wake the selected agent if needed. Deliver through the existing agent connection and admit its lane when eligible.

Propose a separate installation coordinator connection/model setting, using the same provider adapter implementation. It has no tools or memory pool. Provider failure leaves routing waiting with the message retained; do not silently pick the only member or switch models.

### Delegation and transfer

Creating a child locks the relevant cancellation/ancestry state, verifies grants, records root and parent, and creates delivery in one transaction. Reject cycles and excessive depth. Proposed initial maximum depth is eight, with a visible limit error; this is a safety proposal, not a hidden monetary budget.

Parents may yield with dependency IDs. Child outcome publication records result and schedules a deduplicated parent continuation. A transfer increments ownership revision and waits for the old execution to settle before a new owner starts. It does not create a fresh allowance or erase history.

### Steering and cancellation

Steering is a durable targeted control request. Runtime acknowledgement identifies the context checkpoint where it was incorporated. If the task already ended, return not-applied-to-terminal-work; do not open another task automatically.

Cancellation records an ancestor marker before descendant delivery. New claims and delegations check that marker transactionally. Runtime requests to start new tools check locally received cancellation and finite authority. Cancellation is idempotent. Late child results are retained as evidence without restarting cancelled work.

### Failure recovery

A failed child reports its typed reason and partial output. The parent can retry a safe operation, choose a different permitted method, reassign, disclose a limitation, or ask for help. Proposed loop guard: after three equivalent failed recovery attempts with no new evidence or changed approach, block and escalate. Record equivalence inputs rather than asking a model alone to decide whether a loop occurred. Provider waiting and explicit cancellation are not ordinary child failures to work around.

## 6. Runtime, invocation, and journal contracts

The provider interface should preserve useful variation:

~~~text
ProviderAdapter:
  capabilities(connection, model) -> ModelCapabilities
  stream(InferenceRequest, cancellation) -> stream<ProviderEvent>

InferenceRequest:
  model, ordered_content, tool_schemas, output_budget, operation_id
ProviderEvent:
  TextDelta | ToolCallDelta | ToolCallComplete | Usage | Stop | Failure
~~~

The adapter never invokes tools. The engine waits for a complete, validated tool call before dispatch. Preserve provider-specific continuation data as opaque, appropriately protected adapter state when required; do not assume portability between models or expose hidden reasoning as a product report.

A tool request carries execution identity, operation ID, schema revision, checked arguments, resource claims, deadline, and authorization evidence. The executor returns accepted/running status separately from completed output. Use authenticated local transport, preferably a Unix socket with peer identity checks where feasible, plus invocation tokens bound to runtime epoch. Socket placement and browser process users must be qualified against the shared pod.

Before issuing an external effect, durably record intent in the runtime journal and, while connected, acknowledge it through the orchestrator. Capture outcomes before claiming completion. The proposed local journal is a bounded, checksummed append log with sequence numbers, atomic checkpoint replacement, and file synchronization; it is not a second authoritative agent database. Local durability must be qualified on the workspace storage class.

| Invocation state | Recovery rule |
| --- | --- |
| Prepared, not dispatched | Can dispatch only with valid current authority |
| Dispatched, no known outcome | Reconcile; timeout alone does not authorize retry |
| Succeeded with result | Deduplicate report and return recorded outcome |
| Failed with known no-effect result | Retry only under the operation's policy |
| Cancel requested | Stop new actions; settle the actual invocation |
| Outcome unknown | Preserve evidence and block unsafe replacement |

Classify each native or integration operation as read-only, idempotent with supported key, reconcilable, or non-replayable. Arbitrary shell is non-replayable by default. Do not trust MCP metadata alone as proof of idempotency. Use provider-native idempotency only where verified.

Workspace resource claims cover shell invocations, installations, native file writes, and known background processes. The initial safe default serializes opaque workspace mutation. Separate browser sessions can operate concurrently, but shared downloads and files still participate in workspace coordination. Unsupported concurrent mutation is queued or denied explicitly.

## 7. Memory, context, and skills

Proposed memory interface:

~~~text
search_memory(agent_context, query, filters, result_budget)
read_sources(agent_context, source_refs, byte_budget)
stage_memory_changes(turn_id, expected_revisions, changes)
commit_successful_turn(turn_id, checkpoint, staged_changes, epoch)
correct_memory(human_context, memory_id, expected_revision, correction)
~~~

Store memory revisions and source links, with status active/superseded/suppressed. Successful-turn commit and memory staging are atomic in PostgreSQL; filesystem and external effects are not rolled back by a rejected memory commit. Conflict responses preserve successful external-action evidence and require reconciling the memory candidate rather than replaying the turn.

Propose lexical candidate retrieval using PostgreSQL text search and exact identifier matching, vector candidates using pgvector, followed by reciprocal-rank fusion and bounded results. Filter by authenticated agent ownership before retrieval and final ranking. Begin with exact vector search for qualification-sized pools; add approximate indexes when measured retrieval cost justifies them, while preserving recall under filters.

Choose the embedding model through installation configuration and record its version/dimension on every vector. Changing it queues reembedding without mixing spaces. Index updates follow memory commits through durable work records. A pending or failed index does not lose the memory; search reports lexical-only degradation. Proposed ready-state qualification requires a functioning embedding configuration, with explicit degraded operation otherwise.

The engine does not automatically insert retrieved memories. Standing instructions, current task, recent conversation, installed skill metadata, and a context checkpoint are normal context. Memory results appear only after an explicit search/read tool request.

Proposed compaction trigger: reserve space for the maximum permitted tool result and selected model output, then compact when projected input exceeds 75 percent of usable context. Preserve tool call/result pairs, requirements, constraints, pending operations, and source references. Verify summary integrity before replacing a checkpoint. If compaction fails or cannot fit, wait visibly instead of dropping instructions or changing the selected model.

Automatic extraction uses source outcomes and version checks. Propose coalescing completed-turn extraction work per agent, with a bounded backlog and reserved maintenance capacity. It does not require the broad idle-only dream gate, but cannot overwrite newer corrections.

Proposed dreaming eligibility: at least five minutes idle, no eligible queued work, and changed source material since the last processed watermark. A periodic check may wake a sleeping agent, initially at most once per day per agent. No changed material means no inference. Yield between bounded batches, commit changes with source/revision checks, and show actual changes in the dream report. These cadence values are tunable engineering proposals.

Skills follow immutable library revisions and per-agent pinned installations. New skill creation can publish and install on the contributing agent; publication success and installation failure remain separate outcomes. Revisions to shared skills do not automatically update existing installations. Concurrent publication uses expected parent revision.

Package imports validate paths, archive expansion, metadata, encoding, and supported file types, and reject escaping paths and unsafe links. Git imports pin a commit and subtree without executing repository hooks or scripts. Import provenance records origin and license metadata where supplied. Library contribution does not grant access to protected source histories.

## 8. Connections, tools, browser, and triggers

Keep provider connections distinct from integration definitions, external account connections, and per-agent assignments. Connections remain installation-owned. Under [ADR 0081](../adr/0081-installation-wide-default-connection-access.md), use access_mode = all_agents by default, or explicit_grants when restricted. All-agents mode dynamically authorizes current and future eligible agents to use the connection's approved tools and resource scope. It does not require per-agent assignment records.

Proposed effective-access evaluation: the connection must be enabled and valid, the agent eligible, and either the all-agents baseline or a valid specific grant must authorize use. Intersect that authority with the connection's approved scope, applicable execution restrictions, upstream credential scope, and separately required network grants. Specific-grants mode denies access without a valid grant. A restricted grant cannot negate a broader baseline; choose specific-grants mode when only a subset should have access.

Administrators choose the access mode and approve tool/resource scope. Delegated managers may assign specific grants within their authority. Connection policy edits increment a revision, update eligible catalogs, invalidate stale authorization decisions, and stop new credential issuance to removed recipients. Switching to specific grants retains only explicit grants, never copied defaults. Apply the same evaluation to provider selection, tool discovery, browser account selection, and invocation. Bounded disconnected execution and already-issued effects retain their existing limitations.

The connection editor exposes Enabled and Available to all agents, with the latter on for new connections; when off, show the specific-grants editor. Explain effective access on an agent as inherited from the installation default or explicitly granted. Broad availability does not start local MCP processes, select an account/model, or preload every schema or secret. A server advertising a new tool does not approve it; once approved, the current connection audience can discover it.

Proposed core includes memory.search/read/write, tools.search/load, skills.search/read/create/revise/install, messaging.send, tasks.delegate/transfer/result, agents.create when granted, triggers.manage, browser operations, terminal execution, and artifacts.publish/read. Tool schemas are versioned and loaded on demand. Search catalogs use normal indexed metadata initially; semantic tool search is not required to fulfill explicit discovery.

MCP protocol sessions do not own task identity. Local stdio clients supervise subprocesses in the executor; remote Streamable HTTP clients use approved endpoints and scoped credentials there. Disable unsupported server-initiated sampling, unsolicited execution, and optional protocol features until explicitly qualified. A session reconnect never proves an earlier write failed.

Browser sessions have named owners, task scope, account grants, input generation, and recovery state. Main and task sessions use separate profiles or contexts; reuse only authorized login state through a controlled, versioned account mechanism. Copying all cookies from another profile is not an authorization strategy. Some sites will require separate reauthentication; represent that limitation.

Takeover uses a session input generation: stop admitting agent inputs, drain or reconcile in-flight input, acknowledge human control, then accept human events. On return, discard queued stale events and require a fresh observation. Viewer traffic is bounded independently from chat. Proposed initial transport is a throttled screenshot stream and authenticated input messages; optimize encoding only after measuring bandwidth and latency.

Trigger creation is ordinary autonomy within existing scope. Administrators still register event sources and integrations. Propose explicit grouping selection in the UI and individual processing as the command default, with bounded queues and visible overflow errors. Persist each schedule occurrence by trigger and scheduled instant, with timezone and DST policy recorded. After downtime, enumerate missed occurrences up to an explicit catch-up bound and report excess; do not flood execution or silently skip an unbounded gap.

## 9. Human access and frontend interfaces

Proposed fixed roles and scopes:

| Role or capability | Default authority |
| --- | --- |
| Installation administrator | Configure platform and grants, inspect all content; credential values are not exposed by content-reading interfaces |
| Member | Access granted resources and contribute to the shared skill library; no automatic private-agent access |
| Resource reader | Read the explicitly granted conversation, memory, artifact, or workspace scope |
| Conversation participant | Read/send in its scope; no implicit memory edit or browser control |
| Agent manager | Manage selected agent configuration and assignments within delegable grants |
| Explicit capabilities | Memory edit, workspace write, browser view/takeover, goal acceptance, destructive deletion |

These are fixed permission names, not user-defined roles or a policy language. Combine applicable positive grants, then enforce inactive identity, disabled resource, installation constraints, and scope. Grants cannot delegate more authority than their grantor may assign. A manager's agent configuration changes do not reveal its provider secrets.

Use built-in accounts with a maintained password-hashing implementation and server-side sessions. OIDC validates issuer, subject, audience, state, nonce, and code exchange. Invitation or configured OIDC auto-admission creates a Member. Do not link accounts solely by email or revive suspended identities on login. Teams are local Cubebot records.

Proposed wire surface:

| Interface | Purpose |
| --- | --- |
| /api/v1/agents, /rooms, /conversations | Authorized management and paginated reads |
| /api/v1/messages and /operations/{id} | Idempotent receipt and uncertain-response reconciliation |
| /api/v1/goals/{id}/commands | Steer, cancel, submit, accept, request changes with typed command schemas |
| /api/v1/memories, /skills, /artifacts | Domain-authorized knowledge and file operations |
| /api/v1/events | Authorized SSE with cursor and reset/snapshot recovery |
| /runtime/v1/connect | Outbound agent control WebSocket, incarnation-bound |
| /browser/v1/sessions/{id} | Scoped view/control channel after explicit authorization |

Opaque IDs, page cursors, timestamps, resource revisions, and operation IDs are consistent across interfaces. Enforce body and output bounds. Use one command error schema and generate TypeScript contracts from versioned OpenAPI. Do not put domain authorization in generated clients.

Use Secure, HttpOnly cookies and CSRF/origin protection for browser mutation channels. Same-origin SSR forwards sessions; Rust remains the authorization owner. Scope query caches to user and installation and clear them on logout or access changes. Recheck stream access and stop revoked subscriptions; avoid sensitive content in push payloads.

SSE reconnect resumes from durable event cursors where available; an expired cursor instructs the client to refetch an authorized snapshot. Token deltas may be transient, but committed messages and state transitions survive reconnect. The UI labels interrupted partial output and never treats a missing delta as a missing committed task.

Desktop navigation lists agents and rooms, conversation in the center, and on-demand detail panels. Mobile uses a conversation screen and navigable details. Preserve scroll anchors and drafts, bound rendered history, and isolate heavy viewers. Draft storage is keyed by installation, user, and conversation and cleared or made inaccessible on logout according to the explicit account-switching contract. Service workers cache application assets, not authenticated history by default. Pending sends, server receipts, queued assignments, active work, dreaming, provider waits, review, and cancellation settlement have distinct states.

## 10. Proposed relational model and constraints

| Table group | Key constraints and indexes |
| --- | --- |
| identities, login_bindings, teams, memberships, grants | Unique issuer/subject; membership and scope indexes; no email-only linking |
| agents, agent_revisions, incarnations, managed_resources | Stable agent ID; unique current incarnation; recorded Kubernetes UID/ownership |
| conversations, messages, routing_requests | Unique conversation sequence and actor request ID; keyset pagination |
| goals, tasks, dependencies, lanes, turns | Root/parent foreign keys; ownership revisions; no cycles; one active turn per lane |
| invocations, checkpoints, journal_receipts | Unique execution operation and sequence; outcome versus dispatch state separate |
| submissions, requirement_results, reviews | Immutable submission revision; acceptance targets current goal requirements |
| memories, memory_revisions, sources, memory_sources, embeddings | Agent ownership indexes; source/revision foreign keys; embedding model/dimension separation |
| skills, skill_revisions, agent_skill_installs | Unique revision identity; CAS publication; per-agent pinned revision |
| connections, secret_versions, integration_defs, tool_catalog | Access mode and policy revision; auth revision distinct from encryption-key version; endpoint and approved tool schema identities |
| triggers, occurrences, pending_groups | Unique source occurrence key; one open combined group per trigger |
| uploads, blobs, artifact_refs, deletion_jobs | Ready-before-reference invariant; serialized finalize/delete; protected backup references |
| budget_limits, reservations, usage_reports | Root attribution; unique report operation; unresolved reservations not reusable |
| operations, outbox, audit, notifications, maintenance_runs | Idempotent commands; indexed delivery status; audit/resource authorization; durable maintenance state |

Keep heavy bytes outside PostgreSQL, while retaining bounded searchable text and references. Store timestamps in UTC and retain schedule timezone separately. Use foreign keys, uniqueness, and transaction locking for invariants; validate state transitions in the owning module. Exact schema and migration syntax are implementation work.

## 11. Storage, deletion, and recovery

Use the existing [storage proposal](shared-storage.md) for immutable publication and crash reconciliation. Stream files with bounded buffers and independent admission. Domain references authorize downloads; a blob digest or ID alone does not. Serve untrusted HTML and executable content as downloads or on an isolated origin, never with console privileges.

Proposed deletion contracts distinguish workspace file removal, memory suppression, source-record erasure, artifact removal, and whole-agent deletion. Preview affected domains before a destructive user operation; this is product behavior to design, not a requirement for another architecture approval question.

For source erasure, propose invalidating dependent summaries and embeddings and suppressing automatic re-extraction from deleted evidence. Rebuild memories from remaining independent sources where possible. Copied external documents and independently published skill revisions need explicit scope; do not claim universal erasure. Reject stale writes that reference deletion tombstones. Active and disconnected contexts adopt suppression at their next authorized boundary; immediate forgetting across offline executors cannot be promised.

Backups have their own retention and may contain deleted data. Preserve deletion records needed by restore and report the restored recovery point. Do not enable a restored installation until reconciliation accounts for later deletion/cancellation knowledge available to the operator.

Maintenance states are preparing, draining, quiesced, capturing_or_upgrading, verifying, resuming, complete, and failed_with_blockers. Only one maintenance owner operates at a time. Writers acknowledge barriers; forced process death is not a clean checkpoint. For the simplest initial protocol, reject new mutations with a retryable response during quiescence and migrations, retain drafts locally, and serve safe read-only status where available.

Propose full coordinated backups initially, to an administrator-configured destination outside the source volumes, using a database-native consistent dump and qualified volume snapshots or verified file copies. The manifest records database/schema release, agent and shared-file captures, digests where applicable, key versions, and restore instructions. Snapshot creation alone is not proof of a recoverable backup. Test a restore into an isolated namespace before calling a backup workflow qualified.

Upgrade tooling checks a supported release path, establishes the barrier, obtains required recovery captures, runs migrations, replaces compatible artifacts, validates state, and reopens eligible admission. Restore and repair commands must work without the ordinary console. Rollback is release-specific; old images are not a substitute for data rollback.

## 12. Observability, budgets, and control limits

Expose durable user-facing task progress and concise evidence reports. Correlate logs/traces by installation, agent, goal, task, turn, invocation, and operation IDs. Measure admission delay, routing latency, provider wait, tool execution, context size, retrieval latency/quality, dream backlog, storage pressure, and recovery state. Redact credentials and sensitive payloads; content audit access follows resource grants.

Monetary limits are unset by default. When enabled, allocate conservative bounded reservations before direct provider calls, reconcile reported or estimated usage once, and retain uncertain reservations. If usage cannot be bounded for a provider, reject strict enforcement configuration or clearly expose an estimate-based mode; do not promise an exact invoice cap. Tighter child limits do not create additional root allowance.

Hardware, concurrency, capture limits, and disconnected authority still constrain execution when no money limit is configured. Ordinary trigger autonomy cannot raise these limits. Dreaming and routine maintenance have explicit independent attribution rather than borrowing the last user goal's identity.

## 13. ADR coverage

Every accepted ADR is covered by the architecture, this specification, and the qualification plan. The ranges below are navigation groups; each ADR remains individually authoritative.

| Accepted decisions | Consolidated contract |
| --- | --- |
| 0001–0004 | Installation, ownership, administrator access, unified memory: architecture and sections 2, 7, 9 |
| 0005–0013 | Lanes, engine, lifecycle, persistence, orchestrator, recovery, autonomy: sections 2–6 and 12 |
| 0014–0019 | Rooms, messaging, creation, lifetime, briefing: sections 2, 3, 5; architecture collaboration |
| 0020–0027 | Source-preserving memory, retrieval, compaction, dreaming, correction: section 7 and deletion in 11 |
| 0028–0032 | Provider connections, model policy, direct inference and refresh: sections 6, 8, 12 |
| 0033–0039 | Runtime/tool separation, installation, network, browser and takeover: architecture trust and section 8 |
| 0040–0048 | MCP, registry, accounts, restrictions, discovery, triggers: section 8 |
| 0049–0054 | Accounts, admission, grants, teams, PWA and notifications: section 9 |
| 0055–0060 | Skills, imports, shared storage and native interface: sections 7, 10, 11 |
| 0061–0068 | Qualification, database deployment, backups, keys, budgets and pauses: architecture, section 12, qualification plan |
| 0069–0073 | Frontend, conversation workspace, steering, cancellation and failure: sections 3, 5, 9 |
| 0074–0077 | Database-owned definitions, namespace, runtimes and upgrades: architecture and sections 4, 11 |
| 0078–0080 | Providers, retention and completion: sections 3, 6, 7, 11 |
| 0081 | Installation-wide default connection access with specific-grants mode: sections 2, 8, 10 |

The [ADR index](../adr/README.md) provides the individual source links. The qualification plan records remaining evidence gaps and the order in which to resolve them.
