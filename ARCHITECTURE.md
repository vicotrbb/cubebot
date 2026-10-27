# Cubebot system architecture

Design baseline: 2026-10-24, updated 2026-10-25 for connection access and repository scaffolding. Status: consolidated architecture proposal grounded in 82 accepted ADRs. The repository now contains identity/lifecycle foundations and a minimal native text-chat implementation and a gated terminal implementation. The broader agent system described below remains intended design. See [README](README.md) for the actual runnable scope.

The [manifesto](MANIFESTO.md) states the product goal. The [ADR index](docs/adr/README.md) is authoritative for accepted choices. This document connects those choices into a system; implementation selections explicitly called proposals remain reviewable. The [software specification](docs/design/software-specification.md) defines proposed types, interfaces, states, and workflows. The [qualification plan](docs/design/qualification.md) defines evidence needed before shipping.

## Start with the experience

Each agent has exactly one permanent conversation. Selecting it opens that continuous history, and all context management happens within it. There are no separate direct-chat sessions for an agent. See [ADR 0082](docs/adr/0082-one-permanent-conversation-per-agent.md).

A person opens Cubebot, selects an agent or room, and asks for useful work. Cubebot records the request before acknowledging it. A sleeping agent wakes, receives an authorized assignment, retrieves memories or discovers tools when needed, and works through its own Rust loop. It can delegate and yield while waiting. Results, artifacts, and failures retain their task relationships.

The person sees progress, can steer a particular assignment, and can stop a goal and all its descendants. Agent completion is the default; an explicitly review-required goal waits for human acceptance of a particular result revision. When idle, the agent can dream, improving memories and skills with visible committed changes. The identity, history, and workspace outlive the pod.

The first production target is 100 registered agents with representative retained data and 25 agents making sustained, overlapping progress on mixed organizational work. Thousands of agents remain the ambition, not a measured capability. Human interaction must remain responsive during this work.

## Deployment shape

~~~mermaid
flowchart TB
    U["Desktop browser / mobile PWA"] --> I["HTTPS entry point"]
    subgraph NS["One Cubebot namespace"]
      I --> F["TanStack Start frontend · Bun"]
      I --> O["One active Rust orchestrator"]
      F --> O
      O --> DB[("PostgreSQL")]
      O --> S[("Shared-file PVC")]
      O --> K["Kubernetes lifecycle module"]
      subgraph A["One pod per running agent"]
        R["Trusted Rust runtime"] --> T["Rust tool executor"]
        T --> B["Browser · terminal · local MCP"]
        T --> W[("Agent workspace PVC")]
        R --> J[("Private recovery journal")]
      end
      R --> O
      O --> R
    end
    K --> API["Kubernetes API"]
    API --> A
    R --> P["OpenAI · Anthropic · compatible endpoints"]
    T --> X["Authorized websites / integrations"]
    O --> P
~~~

The orchestrator calls models only for its own duties, such as room coordination and proposed embedding work. Ordinary agent inference goes directly from its runtime to the provider. Arrows show logical communication; the agent opens the proposed control connection to the orchestrator.

Accepted deployment constraints:

- One individual or organization per installation, with installation-owned agents and scoped managers.
- One namespace for all namespaced installation resources. External PostgreSQL and cluster-scoped infrastructure retain their own placement.
- One Rust orchestrator process and one active instance, accepting coordination downtime during restart.
- One pod per running agent, containing separate trusted runtime and tool containers.
- Shared PostgreSQL for structured records; separate persistent workspaces and an orchestrator-owned volume for shared files.
- A dedicated TanStack Start frontend with Bun and Vite, serving desktop and installable PWA.
- Standard container runtime by default; approved optional sandbox runtimes, enforceable by installation policy.

Proposed Kubernetes implementation: deploy the orchestrator with one replica and Recreate rollout behavior, the frontend independently, and bundled PostgreSQL as a single-instance StatefulSet. Manage agent Pods directly from database intent, using separately retained PVCs. An additional Deployment or StatefulSet per agent is unnecessary for the selected initial lifecycle: the orchestrator already owns sleep, wake, retirement, and recovery. A failed node's pod replacement waits for safe reconciliation; it is not a promise of uninterrupted execution during orchestrator downtime.

Use Helm for installation packaging, including an external-database mode. Prefer a qualified CSI volume mode that enforces exclusive attachment for the shared store. Neither one replica nor exclusive volume attachment proves old executors have stopped. Fencing remains a separate requirement.

## Small deployment, deep modules

The backend is a modular monolith. Modules own rules and transactions; they are not separate network services. Transport handlers translate requests and call domain operations. PostgreSQL is the concrete persistence implementation, without a generic repository framework.

| Module | Owns | Representative interface |
| --- | --- | --- |
| Identity and access | Built-in accounts, OIDC identities, teams, scoped grants, effective access | authorize(actor, action, resource); explain_access(...) |
| Agents and lifecycle | Agent definitions, configuration revisions, runtime policy, pod and workspace ownership | create_agent(...); reconcile_agent(id); retire_agent(...) |
| Work | Conversations, room routing, goals, tasks, mailboxes, dependencies, lane admission, triggers, cancellation, budgets, acceptance | send_message(...); delegate(...); claim_turn(...); submit_result(...) |
| Knowledge | Source lookup, memory revisions, retrieval, context checkpoints, dream reports, skill library and installations | search_memory(...); commit_changes(...); publish_skill(...) |
| Connections and tools | Provider and integration registry, credential refresh, capability catalog, tool policy | resolve_connection(...); search_tools(...); authorize_invocation(...) |
| Storage | Upload lifecycle, immutable blobs, domain references, integrity, controlled reclamation | publish(...); read(reference); reconcile_upload(...) |
| Operations | Maintenance, backup/restore, upgrades, health, audit and user notifications | enter_maintenance(...); capture_backup(...); restore(...) |

Keep task scheduling, messaging, triggers, and budgets inside Work initially because they must agree on admission and root-goal identity. Knowledge can contain private memory and skill submodules with distinct permissions. Extract a new module only when it has an independent invariant or demonstrated implementation variation.

The Rust runtime contains the engine, context assembly, provider adapters, tool dispatch, and recovery journal. The Rust executor supervises processes and browser sessions and adapts MCP. It does not own agent turns or database access.

Accepted source layout, revised on 2026-10-25: three applications under `apps/` (`orchestrator`, `agent`, and `web`) and two shared Rust libraries under `modules/` (`protocol` and `providers`). Agent-specific behavior stays inside the agent app, including its tool runner as a second executable. The runtime and tool runner still run in separate containers under ADR 0033; source ownership does not merge their execution authority. Orchestrator domains remain ordinary Rust modules within its application. Shared libraries never depend on applications. See the [repository layout](docs/development/repository-layout.md) for ownership and development paths. This replaces the earlier four-package layout proposal without introducing new services.

The repository scaffold selects Tokio/Axum for asynchronous HTTP and tracing for diagnostics, with locked dependencies and a pinned Rust toolchain. SQLx for PostgreSQL and kube-rs for cluster interaction remain proposals until those features are implemented. The TanStack Start frontend has a locked Bun/Vite build and a smoke-tested Bun server. These checks establish scaffold compatibility, not performance or agent-system qualification. [Axum](https://docs.rs/axum/latest/axum/), [SQLx](https://docs.rs/sqlx/latest/sqlx/), [kube-rs](https://kube.rs/).

## State ownership and persistence

PostgreSQL owns agent definitions, grants, conversations, work state, mailbox deliveries, memory and skill metadata, credential ciphertext, and shared-file references. Agents never receive database credentials. Kubernetes owns actual resource state; Cubebot observes it without treating pod annotations as authoritative agent configuration.

Use current-state relational tables plus transactional audit/outbox records. This is not full event sourcing. A domain transition and its durable notification are committed together. Internal workers claim small batches and commit before external calls. A polling loop remains correct when an optional notification wake-up is missed. PostgreSQL documents row-locking options useful for queue consumers and NOTIFY as a signal between sessions; the durable queue is our proposed application contract. [SELECT](https://www.postgresql.org/docs/current/sql-select.html), [NOTIFY](https://www.postgresql.org/docs/current/sql-notify.html).

Each agent has a workspace PVC. Proposed layout separates runtime journal and tool-writable directories by mounts and permissions; only the tool home/workspace directories are mounted into the tool container. The runtime never executes code from that writable workspace. Workspace files are mutable; published artifacts and skill bundles are immutable snapshots in shared storage.

The [shared-storage proposal](docs/design/shared-storage.md) defines staging, publication, file synchronization, and crash reconciliation. It provides a native authenticated interface, not S3 compatibility, distributed replication, or a shared writable filesystem for every agent.

Retain durable data until explicit deletion. Retiring or sleeping releases execution resources, not workspace data. Bound caches, staging, diagnostic logs, and capture sizes independently. Capacity exhaustion must block affected writes visibly rather than erase retained evidence.

## Execution and collaboration

Each agent has one serial main lane and a configurable one-to-four concurrent task lanes, default two. A lane is an execution queue, not a transcript. Main-lane work uses the agent's permanent conversation; context assembly manages that conversation and the current assignment without creating another chat session. All lanes share the agent's memory and workspace.

Human room mentions create independent assignments for each eligible target. An unmentioned human request invokes the tool-free coordinator, which returns a validated owner, continuation, clarification, or no-action result. Agent-authored progress does not retrigger routing. Delegation and transfer are structured operations with explicit identities and ancestry.

A parent waiting for children yields its execution slot. Child outcomes update dependencies and make a continuation eligible. Failed children return evidence for parent adaptation while healthy siblings continue. A stop propagates through the selected task subtree. User cancellation cannot be bypassed by reassigning the same work under a fresh identity.

Ordinary messages queue. Steering targets a specific active assignment and is incorporated at a safe boundary. The UI distinguishes accepted steering from incorporated steering. Already-issued effects can still complete.

Proposed resource coordination serializes opaque shell commands and workspace-mutating operations per agent by default, while allowing concurrent inference and independently targeted browser sessions. Native file operations can later use narrower locks after qualification. A background process retains the resource claim it needs until it exits or is explicitly supervised as an active service. Advisory path locks cannot prove that arbitrary shell programs respect declared paths.

Schedules and external events enter the same work system. Each trigger chooses individual assignments or one combined pending group. Preserve event references and deduplication independently of grouping. Maintenance and overload produce explicit outcomes, never silently dropped accepted events.

## Agent loop and provider seam

~~~text
receive authorized turn and execution epoch
load relevant conversation, assignment, checkpoint, instructions, installed skill metadata
repeat:
  check authority, cancellation, steering, budget, and pending outcomes
  compact at a safe boundary if context requires it
  request the selected provider/model
  validate complete tool requests
  authorize, journal, execute, and reconcile each invocation
  commit eligible turn results and staged memory changes
  finish, yield, pause, or continue
~~~

Native OpenAI and Anthropic adapters and a defined OpenAI-compatible protocol subset are initial scope. Capabilities are explicit. Visual tools require suitable image input; unsupported capabilities do not silently downgrade the workflow. A provider outage retries within bounds and then waits for the selected model, without fallback.

The orchestrator owns credential refresh and propagates versioned, scoped inference material. Runtime credentials are absent from tool mounts and model-visible results. Subscription authentication for Codex and Claude remains a qualification gate, not a currently supported capability. Supervising their CLI engines would contradict the accepted native-loop choice.

## Memory and skills

One knowledge pool belongs to each agent across its conversations, rooms, and tasks. Access to a transcript controls who can read that transcript, but does not create a compartment inside the agent's memory. An agent may reuse knowledge across its authorized participation. Organizations needing isolated knowledge must use separate agents; the product must explain this property when assigning rooms and access.

Memory search happens only when the agent asks. Propose PostgreSQL full-text search plus pgvector, ownership-filtered candidates, and reciprocal-rank fusion. Store embedding model and revision metadata; never compare incompatible embedding spaces. Use an explicitly configured installation embedding connection, separate from inference defaults. If unavailable, report lexical-only degraded retrieval and queue reindexing rather than claim hybrid retrieval succeeded. [pgvector](https://github.com/pgvector/pgvector).

Rolling compaction maintains a bounded context with recent turns and source-linked summaries. Explicit memory writes are staged to successful turn completion; selective extraction records source outcomes. Automatic maintenance must respect human corrections and stale-write checks. Dreaming runs only when all lanes are idle, yields to new work, and publishes reports tied to committed memory and skill revisions.

The shared skill library contains immutable revisions with contributor provenance and optimistic concurrency checks. Any active user or agent can contribute. Installing a revision on one agent does not update other agents. Skills carry instructions and optional supporting files, not additional permissions. Load installed metadata initially, retrieve full instructions on demand, and search the library when needed. Import files, folders, archives, and pinned Git content using the Agent Skills format; importing never runs package scripts. [Agent Skills specification](https://agentskills.io/specification).

## Tools, browser, and integrations

Expose a small native core for memory search, tool discovery, messaging, tasks, skills, browser, terminal, and artifacts. Search returns only eligible tools; invocation rechecks current permissions and resource restrictions. Never rely on a model to obey an unavailable-tool description.

Integrations and connections are installation-managed. Under [ADR 0081](docs/adr/0081-installation-wide-default-connection-access.md), enabled connections default to all-agents access, including future agents. Each connection can instead require specific grants. The baseline includes its approved tools and configured resource scope without individual agent assignments. Administrators still configure integrations and policies; discovery remains on demand. Eligibility does not mean copying every credential into every pod or automatically selecting a provider or account.

Proposed placement: native runtime operations call orchestrator interfaces; browser, shell, and local MCP run through the executor; the executor also hosts remote MCP clients with scoped integration credentials. This gives tool network policy one execution location. The runtime still validates invocation grants, and the remote service's credentials must constrain access where available. Do not infer arbitrary resource restrictions from tool parameter names.

Use one main browser session and separate task sessions, with explicitly reused authorized website logins. Propose browser automation through Chromium's protocol with structured DOM/accessibility tools and screenshot-driven interaction. Keep the executor and protocol implementation in Rust; Chromium and third-party tools retain their own implementations.

Viewing is distinct from control. Human takeover obtains exclusive input ownership for the named session, waits for prior input to settle, and makes the agent reobserve the page when control returns. No queued offline pointer or keyboard replay.

## Trust and failure containment

Human sessions, agent runtime identity, and tool invocation authority are separate credential classes. Proposed runtime bootstrap uses a projected, audience-bound Kubernetes service-account token mounted only in the trusted container. The orchestrator validates the token and pod identity and binds the resulting Cubebot credential to the agent incarnation. Disable automatic token mounting for the tool environment. Projected tokens and audience selection are platform mechanisms; the exchange and fencing contract are Cubebot design work. [Kubernetes projected volumes](https://kubernetes.io/docs/concepts/storage/projected-volumes/).

Encrypt provider and integration records in PostgreSQL using an established authenticated-encryption implementation and a versioned installation key held separately. Propose envelope encryption to separate bulk records from master-key rotation. Passwords use one-way password hashing, not reversible credential encryption. Backups require the corresponding key versions.

Agents may use public internet destinations; private access requires grants. Default-deny workload rules and explicit allowed paths are necessary, but containers within a pod share networking. This creates a real unresolved implementation obligation: arbitrary tool programs must not exploit runtime-only private paths or local control sockets.

The preferred qualification candidate is distinct non-root runtime/tool identities with enforced tool egress at the operating-system layer, plus pod NetworkPolicy and authenticated local interfaces. Compare a bounded network-setup init helper with node-managed cgroup enforcement. The former adds an elevated setup requirement; the latter adds cluster integration. Neither is accepted as proven or silently required by this draft. Qualification must test direct sockets, IPv6, alternate DNS, redirects, UID changes, browser sandbox behavior, and optional runtimes. Proxy environment variables alone fail the contract. If neither candidate meets the selected layout safely, report a design conflict before implementation rather than weakening the grant semantics. [Pod networking](https://kubernetes.io/docs/concepts/services-networking/), [NetworkPolicy](https://kubernetes.io/docs/concepts/services-networking/network-policies/).

Use monotonic execution epochs and finite authority for disconnected work. A stale epoch cannot commit state, but that alone cannot stop an external request already issued. Do not hand an uncertain lane to another executor until reconciliation or qualified fencing makes replacement safe. The same principle applies to single-active orchestrator recovery.

## Console and communication

Propose same-origin routing: page requests go to TanStack Start; domain requests and event streams go to Rust. SSR forwards the user's session to Rust for authorization, never a caller-controlled identity header. Frontend server functions may compose views without becoming another domain backend.

Use typed HTTP/JSON commands, OpenAPI-generated TypeScript contracts, SSE for user-visible state updates, and authenticated WebSocket channels for runtime control and browser interaction where duplex traffic is needed. Domain records remain authoritative; streams are delivery paths, not durable state.

Use route-based SSR, TanStack Query for request state, bounded conversation rendering, local stream updates, and lazy heavy viewers. Choose one coherent accessible component system during UI work. Proposed visual direction is a calm conversation workspace with readable typography, restrained status colors, and dedicated work/evidence panels; final styling requires concrete desktop/mobile designs. Bun and TanStack document deployment and selective rendering mechanisms, which still require a pinned integration test. [Bun guide](https://bun.sh/guides/ecosystem/tanstack-start), [Selective SSR](https://tanstack.com/start/latest/docs/framework/react/guide/selective-ssr).

PWA use is online, with local drafts and optional push. Cache the application shell deliberately, not authenticated histories by accident. Reconnect fetches authoritative state before replaying observations; it never submits drafts automatically.

## Operations and simplicity choices

Use PostgreSQL-backed scheduling and inbox/outbox records instead of adding Redis, Kafka, or a workflow server. Keep application history in relational state instead of reconstructing everything from an event log. The workload does not yet justify a service mesh, separate vector database, S3 compatibility layer, custom agent CRDs, or multiple orchestration services.

Backups and upgrades use coordinated maintenance, verified writer quiescence, and a recoverable cross-store manifest. Capture the database, shared files, workspace volumes, installation metadata, and separately protected key versions. Restoration remains inactive until identity, ownership, references, and unresolved effects are reconciled.

Emit structured logs, metrics, and trace correlation through existing Rust instrumentation. Propose optional OpenTelemetry export and a Prometheus-compatible metrics endpoint; neither requires a bundled observability stack. Keep user-facing progress and audit records durable in PostgreSQL. Do not expose hidden model reasoning or raw credential material in diagnostic views.

## Consequential proposals to review

| Proposal | Chosen draft shape | Alternative and reason |
| --- | --- | --- |
| P01 Work persistence | PostgreSQL state plus outbox and bounded claim workers | External broker/workflow engine adds another authoritative coordination system |
| P02 Agent workloads | Directly reconciled Pods with retained PVCs | Per-agent StatefulSet duplicates part of lifecycle ownership; revisit if it simplifies proven recovery |
| P03 Wire interfaces | HTTP/JSON, SSE, narrowly scoped WebSockets | gRPC everywhere adds browser adaptation without a demonstrated initial requirement |
| P04 Workspace concurrency | Conservative executor resource claims for opaque mutations | Fully parallel arbitrary shell writes cannot be made safe through advisory path hints |
| P05 Retrieval | PostgreSQL lexical/vector candidates with explicit degradation | Separate vector service adds dependency; automatic prompt injection contradicts ADR 0023 |
| P06 Tool networking | Qualify OS-enforced separation inside the accepted pod layout | Plain proxy settings cannot enforce arbitrary-program restrictions; separate pods require revisiting ADR 0033 |
| P07 Availability | Stop on uncertain ownership and reconcile before replacement | Lease expiry alone risks duplicate external effects |
| P08 Deployment | Helm and a bundled single-instance database option | Operator dependency and HA database behavior are not needed for the selected initial availability |

## Read next

- [Software specification](docs/design/software-specification.md): types, operations, state transitions, persistence, detailed flows, and ADR traceability.
- [Qualification and delivery](docs/design/qualification.md): release gates, measured performance targets, failure scenarios, evidence classes, and implementation sequence.
- [Shared storage](docs/design/shared-storage.md): filesystem publication and recovery mechanics.
- [Accepted decisions](docs/adr/README.md): the full set of choices, including the final [completion policy](docs/adr/0080-agent-completion-with-optional-human-acceptance.md).

Implementation may start from the reviewed contracts, but security enforcement, fencing, provider authentication, and durability must earn their support claims through the qualification plan.
