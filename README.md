# Cubebot

A Kubernetes-native home for persistent agents, shared rooms, memories, and skills.

**Status: minimal text-chat MVP implemented and locally validated.** Built-in sign-in/out, authorized agent creation/editing, encrypted OpenAI API-key/model configuration and one permanent text conversation per agent now connects to the native Rust runtime. The website exposes these operations with progress and failure states. See [text-chat setup and evidence](docs/development/text-chat-mvp.md) for exact scope, checks and current limitations. Broader agent-system features remain unavailable.

## Start developing

Use Rust through rustup, Bun **1.4.0**, Node **24.18.0**, uv **0.11.26**, and just **1.55.1**. A C/C++ compiler and linker are needed to build the syntax-aware complexity tool. Rust **1.96.0** and Python **3.14.6** are selected by repository files; uv can provision Python. The Python project and version pin live in `tooling/quality`. Python runs development checks only, not any Cubebot service.

```sh
just bootstrap
just check
just smoke
```

`bootstrap` installs locked Rust, frontend, and development dependencies, and pins Mozilla's complexity analyzer into the ignored `.tools/` directory. `check` runs formatting, linting, complexity enforcement, tests, builds, and TypeScript checking. The frontend build generates its route tree before type checking. `smoke` requires built artifacts and starts temporary local processes, checks them, and stops them.

For development, use two terminals:

```sh
just dev-api
```

```sh
just dev-web
```

Open <http://localhost:3000>. Vite forwards `/api` to the orchestrator at `127.0.0.1:8080`. `GET /api/health/live` returns `{"status":"alive"}`; this is process liveness, not database readiness or a functioning agent engine. The production frontend is a separate server; production `/api` routing will belong to the installation ingress.

The orchestrator accepts exported `CUBEBOT_BIND_ADDR` and `RUST_LOG` variables. It defaults to `127.0.0.1:8080` and `info`; it does not load `.env` itself. Invalid values stop startup. SIGINT and SIGTERM request graceful shutdown.

## Layout

| Path                | Responsibility                                                        |
| ------------------- | --------------------------------------------------------------------- |
| `apps/orchestrator` | Installation control plane; identity, PostgreSQL, HTTP and shutdown   |
| `apps/agent`        | Agent runtime and internal tool runner in one Rust package            |
| `apps/web`          | TanStack Start frontend and future PWA                                |
| `modules/protocol`  | Shared communication contracts                                        |
| `modules/providers` | Bounded native OpenAI text adapter                                    |
| `tooling/quality`   | Complexity gate, regression tests, and Python development environment |
| `tests/system`      | Smoke checks spanning built application processes                     |
| `deploy`            | Installation packaging; Dockerfiles live with their apps              |
| `docs/adr`          | Accepted product and architecture decisions                           |

See [repository layout](docs/development/repository-layout.md) for the complete module map and dependency rules. The agent package builds `cubebot-agent` and `cubebot-tool-runner`. The runtime starts its authenticated control and text-inference loop when configured. Text-chat lifecycle pods launch only that runtime; the pre-existing tool runner is excluded from the MVP and is never launched by that pod compiler. The runtime binary also supports the trusted init-container operation `--initialize-storage runtime|tools`, which creates and verifies its role directory beneath `/storage`. The tool runner belongs to the agent app and will run in its separate tool container.

The backend remains a modular monolith. PostgreSQL uses SQLx with embedded versioned migrations. The agent module also has a namespaced kube-rs API adapter, durable resource reservations and a fixed pod/PVC specification compiler. Controlled HTTP, PostgreSQL and manifest tests cover these boundaries; explicit reconciliation provisions and observes installation anchors, workspace claims and pods, and explicit installation configuration now activates serial lifecycle reconciliation at startup. The compiler requires digest-pinned images, a dedicated runtime service account, HTTPS orchestrator origin and CSI support for ReadWriteOncePod. Its mount/security layout is not cluster-qualified and does not yet enforce distinct runtime/tool egress.

Kubernetes clients are constructed from explicit configuration with transport retries disabled; reconciliation owns safe recovery rather than replaying effectful requests. Explicit reconciliation now drains, requests guarded pod shutdown, retains terminal evidence, releases only its own execution finalizer and permits a later pod epoch. The same recovery path handles a proven terminal pod without advancing the agent's desired generation; partial runtime failure, NodeLost and deletion latch that physical attempt into drain. Semantic reconciliation of recovered execution outcomes and real-cluster fencing qualification remain pending. Authenticated quiescence reports alone never authorize replacement; a missing pod without retained terminal proof remains uncertain.

Runtime bootstrap TokenReview and 120-second identity sessions are enabled with explicit controller configuration. Runtime identity does not grant a turn or permission to invoke tools. The native runtime now bootstraps, renews machine identity, polls control and reports an empty-journal drain before exiting. It requires `CUBEBOT_ORCHESTRATOR_ORIGIN`, `CUBEBOT_BOOTSTRAP_TOKEN_FILE` and a private existing `CUBEBOT_JOURNAL_DIRECTORY`; the journal occupies its `journal` subdirectory. HTTPS is required except for literal loopback HTTP used in local tests. Redirects, automatic retries and environment proxy discovery are disabled. Pending journal records are uploaded through authenticated immutable custody receipts before local cleanup. Unreconciled central evidence holds Active eligibility while retaining finite Recovery access. Custody does not commit an execution outcome or authorize replay. Text turns now use finite inference grants, journal intent/results privately and reconcile replies idempotently into PostgreSQL. No tool execution is admitted.

## Quality rules

**Cyclomatic complexity must be <= 15 per authored function, including tests and tooling.** This is a hard maximum, not a codebase average. `just complexity` scans the repository, including newly added directories. Rust `match` arms, anonymous functions, and the inclusive 15/16 boundary have regression coverage.

See [the quality policy](docs/development/quality.md) for exact tools, scope, exclusions, and limitations. `just format` applies formatting. GitHub Actions runs the same `just check` and `just smoke` commands and separately builds both containers. Hosting the workflow does not enable branch protection; configure required checks when a remote repository is created.

`just container-isolation` builds disposable runtime/tool/qualification images and compares Linux filesystem and network mechanisms in an isolated Docker namespace. The [recorded mechanism results](docs/development/container-isolation.md) favor UID-aware kernel filtering, but the production Pod has no such filter yet. Full G1–G3, CNI/CSI, DNS, grants, browsers and arbitrary-tool qualification remain pending.

## Optional local database

The liveness-only process still runs without a database. To run the identity API with PostgreSQL:

```sh
cp .env.example .env
# Choose your local POSTGRES_PASSWORD in .env.
just db-up
just db-down
```

Compose binds PostgreSQL to loopback and retains its named volume when stopped. The example credentials are for local development. The orchestrator applies embedded migrations when `CUBEBOT_DATABASE_URL` is exported. Invalid configured databases stop startup. The development image follows PostgreSQL major 18; production image digests and pgvector remain unqualified.

### Identity development workflow

Export `CUBEBOT_DATABASE_URL` with your local PostgreSQL connection string. Create the initial administrator through the local operator binary, using exported `CUBEBOT_BOOTSTRAP_LOGIN` and `CUBEBOT_BOOTSTRAP_PASSWORD`, then remove those bootstrap variables from your shell:

```sh
cargo run --locked -p cubebot-orchestrator --bin cubebot-admin -- bootstrap
cargo run --locked -p cubebot-orchestrator
```

Bootstrap is one-time and serialized in PostgreSQL. Passwords must contain 12–1024 bytes and logins use 3–64 lowercase ASCII letters, digits, dots, underscores or hyphens, starting with a letter or digit. Passwords use Argon2id. Sessions last one day and persist only SHA-256 token digests. Send the returned opaque token as `Authorization: Bearer <token>` over HTTPS outside loopback development. No public bootstrap or registration endpoint exists.

The current identity API has `POST /api/identity/login`, `GET /api/identity/me`, `POST /api/identity/logout`, administrator-only `POST /api/identity/accounts`, and administrator-only `POST /api/identity/suspend`. Login and account creation accept `login` and `password`; creation also requires a unique `request_id` and accepts `administrator`, default false. Suspension requires account `id`, `expected_revision` and `request_id`. `GET /api/identity/accounts` returns up to 100 administrator-visible account summaries with activity and revisions; pass the last ID as `?after=<id>` for the next page. Logout removes the session, and suspension blocks subsequent authentication and removes the account's sessions. These endpoints are an initial development slice; enforcement by the future agent/room/content handlers remains pending.

Administrators create a seven-day invitation with `POST /api/identity/invitations`, a `login` and a unique `request_id`. The first response contains its ID, revision and a one-time bearer token; only its digest is stored. Retrying that request returns the same ID/revision with `token: null`. Deliver the token through a trusted channel. `POST /api/identity/invitations/accept` requires that token, a new password and a unique `request_id`, creating only the bound member login. Repeating the original request with its original password returns the recorded account ID while the invitation remains valid and the account active; changing the password conflicts, and a different request cannot redeem the used invitation. `POST /api/identity/invitations/revoke` requires the invitation ID, `expected_revision` and `request_id`. Expired/revoked/used invitations cannot create another account, and inviting an existing suspended login cannot reactivate it. `GET /api/identity/invitations` lists up to 100 administrator-visible invitation statuses/revisions, with `?after=<id>` pagination. If the issuing response is lost, inspect and revoke the recorded invitation, then issue a new request to obtain a new token. Raw tokens are not recoverable. External delivery remains manual.

### Optional OpenID Connect and browser sessions

Export `CUBEBOT_OIDC_CONFIG` as the path to a local JSON configuration file, with a configured database. Installation operators control this file; a remote authentication-management UI is not implemented. Configure the provider to allow this exact callback URI:

```json
{
  "issuer": "https://identity.example.com",
  "client_id": "cubebot",
  "redirect_uri": "https://cubebot.example.com/api/identity/oidc/callback",
  "admit_subjects": ["explicit-provider-subject"],
  "admit_email_domains": []
}
```

If the provider requires a confidential client, export `CUBEBOT_OIDC_CLIENT_SECRET` separately. HTTPS is required except for loopback development. `CUBEBOT_PUBLIC_ORIGIN` optionally specifies the exact browser origin and must agree with the callback origin; OIDC otherwise derives it from the callback. Set this origin even without OIDC to enable built-in browser session cookies. The service does not load these values from `.env` itself.

`POST /api/identity/oidc/start` returns an authorization URL and sets a ten-minute HttpOnly browser flow cookie. Follow the URL, then return through the callback. Cubebot validates state, browser binding, PKCE, nonce, signature, issuer, audience, expiry and any access-token hash before issuing its one-day platform session cookie and redirecting to `/`. The frontend supports built-in sign-in/out; optional OIDC endpoints remain available to API clients. HTTPS session and flow cookies use `__Host-` names, `Secure`, `Path=/` and `SameSite=Lax`; loopback HTTP uses development cookie names. Unsafe cookie-authenticated requests require an exact matching `Origin`; bearer API clients keep their existing authentication path.

New membership requires either an exact admitted subject or an explicitly allowed domain on a verified email claim. Both lists default empty. Userinfo may supply a missing email, with its subject checked against the ID token. Admission creates an ordinary member; provider role/group claims confer no administrator status or local team membership. Existing active bindings can sign in without repeating first-login admission. Accounts are bound by trusted issuer and subject, never merged by matching email.

`POST /api/identity/oidc/link` starts explicit linking from a platform session created within ten minutes. That session must still be valid at callback. Each account has at most one active subject per issuer. Administrators inspect bindings with `GET /api/identity/oidc/bindings` (100 records, `?after=<id>` cursor), and revoke with `POST /api/identity/oidc/revoke`, supplying `id`, `expected_revision` and `request_id`. Revocation retains the binding as a tombstone and removes the account's OIDC-derived sessions, while built-in sessions remain independent. Suspension also prevents automatic re-admission. Re-admitting a revoked subject is deliberately unavailable in this slice.

Changing the non-secret OIDC configuration or removing it advances a persisted policy revision, invalidating pending flows and OIDC sessions. Rotating the client secret alone does not invalidate platform sessions. Built-in credentials remain available for installation recovery. Discovery/JWKS are refreshed during callback; provider requests have deadlines, reject redirects and limit response bodies to one MiB. Four concurrent OIDC operations and 1,000 pending unexpired flows bound basic resource use. This slice is tested with a controlled signed issuer; no external identity-provider compatibility or production security qualification is claimed.

### Durable agent definitions

`POST /api/agents` accepts `request_id`, `name` and `instructions`. Creation currently requires an installation administrator and records a durable definition, command receipt and event in one PostgreSQL transaction. Names must contain 1–255 UTF-8 bytes without control characters; instructions accept at most 65,536 bytes and exclude null bytes. Repeating the same request returns the original ID/revision; changing its content conflicts. Current administrator authority is checked before returning a cached creation receipt.

`GET /api/agents` lists up to 100 definitions visible through the current administrator role or an exact `agent_manage` user/team grant. Pass the last returned ID as `?after=<id>` for the next page. Authorization filtering occurs before the page limit. `GET /api/agents/<id>` returns the stored management definition, including instructions, only to those same callers. Unknown and inaccessible IDs both return 404. Revoking a grant or removing team membership takes effect on subsequent reads without a new login. These management permissions confer no memory, workspace or conversation access.

Definitions belong to the installation; `created_by` is provenance rather than ownership. They begin with revision 1 and `desired_state: "sleeping"`, with `idle_policy: "sleep"`. Desired state is persisted intent, not an observation of a pod. Provider configuration and the native text loop are available in the MVP; other agent behavior remains pending. Explicitly configured lifecycle reconciliation can provision the fixed workload layout. No create response claims successful provisioning.

Managers edit with `POST /api/agents/<id>/definition`, supplying `request_id`, `expected_revision`, `name`, `instructions` and `idle_policy` of `sleep` or `keep_running`. Edits increment the agent revision, retain immutable snapshots and increment `instructions_revision` only when instruction text changes. `GET /api/agents/<id>/revisions?after=<revision>` returns up to 100 authorized configuration snapshots in ascending order. Their revision numbers can skip lifecycle-only changes. Existing definitions are snapshotted when this migration is applied; unavailable historical text is not reconstructed.

`GET /api/agents/<id>/status` requires current agent-management access and reports desired state separately from the latest physical attempt, resource phase, validated Kubernetes phase/readiness/reason, observation time/age, blocked reason, drain report and retained terminal-evidence indicator. It includes workspace reservation status and current runtime identity eligibility without credentials, manifests or raw proof. Readiness and identity eligibility do not establish working turns or tool authority; observation timestamps describe retained facts, not a live stream. Missing Running observations hold Active identity while recovery reporting remains bounded by its credential expiry.

`POST /api/agents/<id>/lifecycle` accepts `request_id`, `expected_revision` and `action` of `wake`, `sleep`, `restart` or `retire`. HTTP 202 acknowledges persisted intent only. Wake from sleeping and explicit restart advance `restart_generation`; wake while already running does not. Replay returns its original receipt without repeating the transition. A retired definition remains inspectable but cannot be edited or newly awakened. Previously accepted receipts can still be replayed by their original actor while that actor retains current management access. `keep_running` applies to automatic idle shutdown after waking; a manual sleep preserves that policy. With explicit controller configuration, startup acquires installation process ownership and reconciles these commands automatically. Without it, database-only development mode persists intent and serves no runtime routes. Reconciliation handles provisioning, drain and guarded termination; live-cluster fencing and active-work drain qualification remain pending.

The Kubernetes boundary currently supports namespaced pod/PVC creation, recorded-pod observation and guarded pod stop/finalizer requests. It checks the recorded namespace/name/UID, installation/agent/attempt labels and installation ConfigMap controller anchor before changing an existing pod. Stop requests require Cubebot's execution finalizer and send UID plus resource-version preconditions with a nonzero grace period. Finalizer release requires deletion already requested, non-restarting runtime/tool containers with known identified exit reports, and terminal init containers; it preserves other controllers' finalizers. Missing objects, changed UIDs, unknown container status, NodeLost and ephemeral execution remain uncertain. These are local API-contract tests, not proof of physical fencing. The installation anchor/UID registry, resource specifications, runtime control protocol and configured controller activation are implemented. Real-cluster qualification remains pending. The running service contacts Kubernetes only when explicitly configured. Its single-active guard retains ownership across database-session loss and requires exact prior-container termination evidence for crash takeover.

### Runtime recovery evidence

`POST /api/runtime/journal` accepts one strict `sequence` and byte-array `payload` using finite Recovery credentials. Payloads are bounded to 64 KiB, with a 272 KiB HTTP-body bound for JSON encoding. The per-agent sequence must advance contiguously; retries of an older sequence must contain identical bytes. PostgreSQL retains the payload, digest, first reporting attempt and receipt time. The reply binds the current runtime actor, sequence and SHA-256 digest. Human credentials cannot submit records and the endpoint returns no retained payload.

The native loop checks control before each upload and verifies the entire receipt before synchronizing its local acknowledgement checkpoint and removing that record. Successful backlog progress immediately proceeds to the next control boundary. Errors retain evidence and the existing five-second retry delay. A clean drain report requires no locally pending records. The central copy remains unreconciled and holds Active identity eligibility until future typed outcome reconciliation accepts it; uploading opaque bytes does not resolve unknown external effects. The scoped agent status response exposes `recovery_records_pending` without returning evidence content.

This stream depends on the existing per-agent workspace and durable journal checkpoint. It does not reset sequence numbers after Pod replacement or reconstruct a lost checkpoint from a server counter. Conflicts, missing committed files and uncertain outcomes require recovery rather than silent reset. Typed invocation/result reconciliation, journal crash-window qualification on CSI and finite per-turn authority remain unfinished.

### Explicit lifecycle controller configuration

Set `CUBEBOT_CONTROLLER_CONFIG` to a local JSON file and configure `CUBEBOT_DATABASE_URL`. Supply `CUBEBOT_POD_NAMESPACE`, `CUBEBOT_POD_NAME` and `CUBEBOT_POD_UID` from the orchestrator Pod's Downward API metadata. The observed Pod must have one regular container named `orchestrator`, RestartAlways, a dedicated service account and no ephemeral containers. The ownership proof covers that container; init containers must not run independent orchestrator instances.

```json
{
  "connection": { "kind": "in_cluster" },
  "runtime_image": "registry.example/cubebot-agent@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
  "tools_image": "registry.example/cubebot-agent@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
  "runtime_service_account": "cubebot-runtime",
  "orchestrator_origin": "https://cubebot.example",
  "network": {
    "infrastructure": ["10.244.0.0/16", "10.96.0.0/12", "192.0.2.10/32"],
    "dns": ["10.96.0.10"]
  }
}
```

Replace the example topology with the installation's actual Pod, Service, node, API, metadata, translation and public infrastructure destinations. `network` is required; CIDRs must be canonical and use the actual packet address family. IPv4-mapped IPv6 infrastructure prefixes are rejected. Supply one to three resolver IPs. Pods use `dnsPolicy: None` with exactly those resolvers, so use fully qualified hostnames for cluster services. The same addresses are allowed only on TCP/UDP port 53 by the tool policy. The network initializer runs first as root with only NET_ADMIN, no identity/workspace mounts, no privilege escalation and a read-only root filesystem. Cluster admission must permit this explicit setup capability; Cubebot never changes admission policy or starts tools without successful initialization.

Replace both example image digests with the built installation images. Optional `runtime_class` selects an installed Kubernetes RuntimeClass. For an explicitly selected kubeconfig, replace `connection` with `{"kind":"kubeconfig","path":"/trusted/kubeconfig","context":"cubebot"}`. The service never discovers the default context. Kubernetes transport requires certificate-verified HTTPS, with literal loopback HTTP allowed for local controlled workflows. The orchestrator origin supplied to agents requires HTTPS.

For an orchestrator endpoint signed by a private CA, set optional `control_ca_config_map` to an administrator-managed ConfigMap in the installation namespace containing a `ca.crt` PEM bundle. The compiler mounts that key read-only only in the trusted runtime and sets `CUBEBOT_CONTROL_CA_FILE`. Public roots remain trusted; certificate and hostname checks remain enabled. A missing key blocks Pod startup; an unreadable, empty, malformed or oversized bundle blocks runtime startup. The maximum bundle size is 64 KiB. Trust is loaded at runtime process startup, so replace Pods to apply CA changes. The ConfigMap contains public CA certificates, never a CA private key. Runtime HTTPS behavior has local TLS tests; connected-cluster operation remains unqualified.

The controller scans durable agent intent in keyset batches of 32 and waits five seconds after each full scan. It checks process ownership before each reconciliation and bounds each attempt to sixty seconds. HTTP handling proceeds concurrently. Shutdown stops admission, settles handlers and the current reconciliation, then releases ownership. Ownership-check failure stops both loops and retains the claim. Agent drain and physical termination rules still control whether a replacement Pod is allowed.

Without controller configuration, database-only development mode holds the same exclusion lock and a durable development claim. A clean stop permits later controller activation. An unclean development exit retains the claim because there is no Kubernetes container proof for that local process; automatic takeover is deliberately unavailable. Once Kubernetes ownership has been established, omitting controller configuration is refused. This development mode is not production recovery qualification. Liveness-only mode without a database remains available.

### Authentication resource limits

The orchestrator permits at most four concurrent Argon2 computations across password hashing and verification. Additional work receives HTTP 429 with `Retry-After: 1`; it does not enter an unbounded hashing queue. The permit stays with the blocking computation if its HTTP caller disconnects.

Each active account has a persisted budget of ten password-verification attempts per 60-second window. Login reserves against that account; password-bearing command replay reserves against the command's actor. Successful and failed attempts both count, including a reservation followed by a busy hash worker. Invitation replay and login share the member's budget, so replay is not an alternate unlimited password-checking path. Exhaustion returns the generic unauthorized response until the window resets. Restart does not clear the budget, and existing sessions remain usable. These basic controls still require broader operational load/security qualification.

### Teams and human permissions

Administrators manage local access teams through `GET/POST /api/identity/teams`, `POST /api/identity/teams/membership` and `POST /api/identity/teams/delete`. Team creation accepts `request_id` and `name`; membership changes accept `request_id`, `team`, `account_id`, `expected_revision` and `present`. Deletion accepts `request_id`, `team` and `expected_revision`. Listing returns at most 100 records; use the last team's ID as `?after=<id>` for the next page. Deleting a team disables its access while preserving its records. Suspended accounts can be removed from teams, but cannot be added.

`POST /api/identity/access/grants` accepts `request_id`, a `recipient` object with `kind` of `user` or `team` and its `id`, a `scope` object with `kind` of `agent`, `room` or `conversation` and its `id`, and a named `capability`. Supported capabilities are `agent_manage`, `conversation_read`, `conversation_send`, `memory_read`, `memory_edit`, `workspace_read`, `workspace_write`, `browser_view`, `browser_control` and `room_manage`. Conversation capabilities accept room/conversation scope; room management accepts room scope; the remaining capabilities accept agent scope. These are exact grants with no implied read/edit/control hierarchy. `POST /api/identity/access/revoke` accepts `request_id`, grant `id` and `expected_revision`.

`POST /api/identity/access/explain` accepts `scope` and `capability` and explains the authenticated user's current access, including direct and team grant sources. Administrators receive these defined permissions through their installation role. Ordinary membership grants no private-resource access, and an agent manager gains no unrelated content or grant-administration authority. Team/grant management is administrator-only initially. Agent and text-conversation handlers check resource existence and content access. Room content workflows remain unimplemented.

Team/grant, suspension and revocation mutation responses include an ID and revision. Their `request_id` is scoped to the authenticated actor and command kind: retries return the recorded receipt, while changed request content returns a conflict. `GET /api/identity/commands/<kind>/<request_id>` reads the caller's receipt, with current administrator authorization. Administrator command kinds are `account.create`, `account.suspend`, `invitation.create`, `invitation.revoke`, `team.create`, `team.membership`, `team.delete`, `access.grant`, `access.revoke` and `oidc.revoke`. These receipts survive restart. Password-bearing commands keep a separate Argon2 verifier to reject changed-password retries without storing a fast password digest. Password verification runs outside database transactions. Administrator receipt retrieval never returns that verifier.

`GET /api/identity/access/events?after=<sequence>` returns up to 100 administrator-visible mutation records, including the actor, affected resource, revision and non-secret request. Publication is serialized through transaction commit so a cursor cannot skip a lower event that commits late. Account/invitation events include only non-secret metadata; passwords, invitation tokens and password verifiers are excluded from their audit payloads. Event streaming and domain-wide notifications remain future work.

`GET /api/health/ready` checks the configured database and exists only in database-enabled mode. It does not attest to agent, Kubernetes or provider readiness.

```sh
just integration
```

This builds the orchestrator/operator binaries, starts an isolated disposable PostgreSQL 18 container with a generated password and temporary host-backed data directory, runs real database/HTTP tests, signed local OIDC issuer workflows and process restarts, then removes that container and its test data. It never uses the database in `.env` or another application's database. Ordinary `cargo test` explicitly reports the database cases as ignored; `just integration` runs them and fails if Docker or PostgreSQL is unavailable.

## Builds and containers

```sh
cargo build --release --workspace --locked
bun run build
bun run --cwd apps/web start
```

The frontend server listens on port 3000 by default. Set `HOST=127.0.0.1` for local-only production preview. Its [documented Bun deployment adapter](https://tanstack.com/start/latest/docs/framework/react/guide/hosting#bun) is Nitro, pinned to `3.0.260903-beta`. This is a prerelease dependency; Bun SSR and static assets are smoke-tested, while production hosting still requires qualification. The current build emits upstream bundler warnings about React Router's `use client` directives; the local browser check found no console errors or warnings.

```sh
docker build -t cubebot-orchestrator:dev -f apps/orchestrator/Dockerfile .
docker build -t cubebot-web:dev -f apps/web/Dockerfile .
```

Container recipes default to distinct non-root runtime/tool users. The runtime image also packages the trusted `--initialize-network` setup command, verified in the [disposable Linux fixture](docs/development/container-isolation.md); the retained full tool layout requires it before tools start. The MVP text-only layout omits both tools and privileged network initialization. Actual Kubernetes/CNI acceptance remains pending. Helm packaging, production network enforcement, full credential isolation and fencing qualification remain open.

Container verification was retried after the layout change on 2026-10-25. Both builds remained blocked by Docker's `no space left on device` error: the orchestrator during dependency compilation and the frontend while materializing its base image. No existing Docker data was pruned. See [layout verification](docs/development/layout-verification.md) for the successful local and fresh-copy checks; image builds remain unqualified.

## Design and next milestone

- [Manifesto](MANIFESTO.md)
- [Architecture](ARCHITECTURE.md)
- [Accepted ADRs](docs/adr/README.md)
- [Software specification](docs/design/software-specification.md)
- [Qualification plan](docs/design/qualification.md)
- [Implementation milestones](docs/superpowers/plans/2026-10-25-cubebot.md)
- [Implementation evidence](docs/development/implementation-progress.md)

The minimal text-chat MVP now connects authorized agent management, durable messages, native execution and restart recovery. Responses commit as complete messages; token streaming and the broader roadmap remain outside this milestone. Cluster security, fencing and scale still require the qualification described in those documents.

`just cluster-admission` runs the [actual Kubernetes admission fixture](docs/development/cluster-admission.md) in an owned disposable kind cluster with PostgreSQL. It requires the verified local kind v0.33.0 tool and tests server defaulting, retained preparation and restricted Pod Security rejection. It creates no running agent or workspace; CSI, CNI and full lifecycle qualification remain pending.
