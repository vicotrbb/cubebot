# Operator and API reference

Configuration and HTTP behavior of what currently runs. Open qualification gaps live in [STATUS](../STATUS.md); intended design lives in [ARCHITECTURE](../../ARCHITECTURE.md).

## Runtime and lifecycle

See [repository layout](repository-layout.md) for the complete module map and dependency rules. Each agent pod runs one container, `cubebot-sandbox --backend <kvm|process>` from `apps/sandbox` (ADR 0084), which runs `cubebot-agent`. The `kvm` backend is not implemented yet and refuses to start; `process` runs the agent as a plain child for development with no isolation claim. The agent runs terminal commands itself when the terminal flag is set.

The backend remains a modular monolith. PostgreSQL uses SQLx with embedded versioned migrations. The agent module also has a namespaced kube-rs API adapter, durable resource reservations and a fixed pod/PVC specification compiler. Controlled HTTP, PostgreSQL and manifest tests cover these boundaries; explicit reconciliation provisions and observes installation anchors, workspace claims and pods, and explicit installation configuration now activates serial lifecycle reconciliation at startup. The compiler requires digest-pinned images, a dedicated runtime service account, HTTPS orchestrator origin and CSI support for ReadWriteOncePod.

Kubernetes clients are constructed from explicit configuration with transport retries disabled; reconciliation owns safe recovery rather than replaying effectful requests. Explicit reconciliation now drains, requests guarded pod shutdown, retains terminal evidence, releases only its own execution finalizer and permits a later pod epoch. The same recovery path handles a proven terminal pod without advancing the agent's desired generation; partial runtime failure, NodeLost and deletion latch that physical attempt into drain. Authenticated quiescence reports alone never authorize replacement; a missing pod without retained terminal proof remains uncertain.

Runtime bootstrap TokenReview is enabled with explicit controller configuration. `POST /api/runtime/bootstrap` issues one agent credential per sandbox incarnation. It has no clock expiry; it stops working when its pod attempt is no longer bound to that pod, which is how it is revoked. The same process may bootstrap again to rotate it. The native runtime requires `CUBEBOT_ORCHESTRATOR_ORIGIN`, `CUBEBOT_BOOTSTRAP_TOKEN_FILE` and a private existing `CUBEBOT_JOURNAL_DIRECTORY`; its tool journal occupies the `tools` subdirectory. HTTPS is required except for literal loopback HTTP used in local tests. Redirects, automatic retries and environment proxy discovery are disabled.

## Local database

The liveness-only process still runs without a database. To run the identity API with PostgreSQL:

```sh
cp .env.example .env
# Choose your local POSTGRES_PASSWORD in .env.
just db-up
just db-down
```

Compose binds PostgreSQL to loopback and retains its named volume when stopped. The example credentials are for local development. The orchestrator applies embedded migrations when `CUBEBOT_DATABASE_URL` is exported. Invalid configured databases stop startup.

### Identity development workflow

Export `CUBEBOT_DATABASE_URL` with your local PostgreSQL connection string. Create the initial administrator through the local operator binary, using exported `CUBEBOT_BOOTSTRAP_LOGIN` and `CUBEBOT_BOOTSTRAP_PASSWORD`, then remove those bootstrap variables from your shell:

```sh
cargo run --locked -p cubebot-orchestrator --bin cubebot-admin -- bootstrap
cargo run --locked -p cubebot-orchestrator
```

Bootstrap is one-time and serialized in PostgreSQL. Passwords must contain 12–1024 bytes and logins use 3–64 lowercase ASCII letters, digits, dots, underscores or hyphens, starting with a letter or digit. Passwords use Argon2id. Sessions last one day and persist only SHA-256 token digests. Send the returned opaque token as `Authorization: Bearer <token>` over HTTPS outside loopback development. No public bootstrap or registration endpoint exists.

The current identity API has `POST /api/identity/login`, `GET /api/identity/me`, `POST /api/identity/logout`, administrator-only `POST /api/identity/accounts`, and administrator-only `POST /api/identity/suspend`. Login and account creation accept `login` and `password`; creation also requires a unique `request_id` and accepts `administrator`, default false. Suspension requires account `id`, `expected_revision` and `request_id`. `GET /api/identity/accounts` returns up to 100 administrator-visible account summaries with activity and revisions; pass the last ID as `?after=<id>` for the next page. Logout removes the session, and suspension blocks subsequent authentication and removes the account's sessions.

Administrators create a seven-day invitation with `POST /api/identity/invitations`, a `login` and a unique `request_id`. The first response contains its ID, revision and a one-time bearer token; only its digest is stored. Retrying that request returns the same ID/revision with `token: null`. Deliver the token through a trusted channel. `POST /api/identity/invitations/accept` requires that token, a new password and a unique `request_id`, creating only the bound member login. Repeating the original request with its original password returns the recorded account ID while the invitation remains valid and the account active; changing the password conflicts, and a different request cannot redeem the used invitation. `POST /api/identity/invitations/revoke` requires the invitation ID, `expected_revision` and `request_id`. Expired/revoked/used invitations cannot create another account, and inviting an existing suspended login cannot reactivate it. `GET /api/identity/invitations` lists up to 100 administrator-visible invitation statuses/revisions, with `?after=<id>` pagination. If the issuing response is lost, inspect and revoke the recorded invitation, then issue a new request to obtain a new token. Raw tokens are not recoverable.

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

`POST /api/identity/oidc/link` starts explicit linking from a platform session created within ten minutes. That session must still be valid at callback. Each account has at most one active subject per issuer. Administrators inspect bindings with `GET /api/identity/oidc/bindings` (100 records, `?after=<id>` cursor), and revoke with `POST /api/identity/oidc/revoke`, supplying `id`, `expected_revision` and `request_id`. Revocation retains the binding as a tombstone and removes the account's OIDC-derived sessions, while built-in sessions remain independent. Suspension also prevents automatic re-admission.

Changing the non-secret OIDC configuration or removing it advances a persisted policy revision, invalidating pending flows and OIDC sessions. Rotating the client secret alone does not invalidate platform sessions. Built-in credentials remain available for installation recovery. Discovery/JWKS are refreshed during callback; provider requests have deadlines, reject redirects and limit response bodies to one MiB. Four concurrent OIDC operations and 1,000 pending unexpired flows bound basic resource use.

### Durable agent definitions

`POST /api/agents` accepts `request_id`, `name` and `instructions`. Creation currently requires an installation administrator and records a durable definition, command receipt and event in one PostgreSQL transaction. Names must contain 1–255 UTF-8 bytes without control characters; instructions accept at most 65,536 bytes and exclude null bytes. Repeating the same request returns the original ID/revision; changing its content conflicts. Current administrator authority is checked before returning a cached creation receipt.

`GET /api/agents` lists up to 100 definitions visible through the current administrator role or an exact `agent_manage` user/team grant. Pass the last returned ID as `?after=<id>` for the next page. Authorization filtering occurs before the page limit. `GET /api/agents/<id>` returns the stored management definition, including instructions, only to those same callers. Unknown and inaccessible IDs both return 404. Revoking a grant or removing team membership takes effect on subsequent reads without a new login. These management permissions confer no memory, workspace or conversation access.

Definitions belong to the installation; `created_by` is provenance rather than ownership. They begin with revision 1 and `desired_state: "sleeping"`, with `idle_policy: "sleep"`. Desired state is persisted intent, not an observation of a pod. Explicitly configured lifecycle reconciliation can provision the fixed workload layout. No create response claims successful provisioning.

Managers edit with `POST /api/agents/<id>/definition`, supplying `request_id`, `expected_revision`, `name`, `instructions` and `idle_policy` of `sleep` or `keep_running`. Edits increment the agent revision, retain immutable snapshots and increment `instructions_revision` only when instruction text changes. `GET /api/agents/<id>/revisions?after=<revision>` returns up to 100 authorized configuration snapshots in ascending order. Their revision numbers can skip lifecycle-only changes. Existing definitions are snapshotted when this migration is applied; unavailable historical text is not reconstructed.

`GET /api/agents/<id>/status` requires current agent-management access and reports desired state separately from the latest physical attempt, resource phase, validated Kubernetes phase/readiness/reason, observation time/age, blocked reason, drain report and retained terminal-evidence indicator. It includes workspace reservation status and current runtime identity eligibility without credentials, manifests or raw proof. Missing Running observations hold Active identity while recovery reporting remains bounded by its credential expiry.

`POST /api/agents/<id>/lifecycle` accepts `request_id`, `expected_revision` and `action` of `wake`, `sleep`, `restart` or `retire`. HTTP 202 acknowledges persisted intent only. Wake from sleeping and explicit restart advance `restart_generation`; wake while already running does not. Replay returns its original receipt without repeating the transition. A retired definition remains inspectable but cannot be edited or newly awakened. Previously accepted receipts can still be replayed by their original actor while that actor retains current management access. `keep_running` applies to automatic idle shutdown after waking; a manual sleep preserves that policy. With explicit controller configuration, startup acquires installation process ownership and reconciles these commands automatically. Without it, database-only development mode persists intent and serves no runtime routes.

The Kubernetes boundary currently supports namespaced pod/PVC creation, recorded-pod observation and guarded pod stop/finalizer requests. It checks the recorded namespace/name/UID, installation/agent/attempt labels and installation ConfigMap controller anchor before changing an existing pod. Stop requests require Cubebot's execution finalizer and send UID plus resource-version preconditions with a nonzero grace period. Finalizer release requires deletion already requested, non-restarting runtime/tool containers with known identified exit reports, and terminal init containers; it preserves other controllers' finalizers. Missing objects, changed UIDs, unknown container status, NodeLost and ephemeral execution remain uncertain. The installation anchor/UID registry, resource specifications, runtime control protocol and configured controller activation are implemented. The running service contacts Kubernetes only when explicitly configured. Its single-active guard retains ownership across database-session loss and requires exact prior-container termination evidence for crash takeover.

### Runtime channel

`GET /api/runtime/channel` upgrades to a WebSocket after checking the agent credential; human credentials are rejected. Messages are JSON text tagged by `type`.

- Agent to orchestrator: `pull` (ready for the next turn), `event` with `turn`, `sequence` and `event`, and `drained` with `request_id`.
- Orchestrator to agent: `turn`, `ack` with `turn` and `sequence` once an event is durable, `cancel` with `turn`, `drain` with `request_id`, and `revoked`.
- A `turn` carries the message, the agent's instructions, the current model and provider key, up to twelve completed earlier exchanges, tool events from earlier runs of the same turn and the first event sequence to use. The agent assembles its own context from these records.
- Event kinds are `text_delta`, `tool_call`, `tool_result`, `usage`, `done` and `error`. The orchestrator appends each to `turn_events`, keyed by turn and sequence; a resent event keeps its first recorded form. Only `done` or `error` from the attempt running the turn settles it. Later events are retained as evidence.

One turn runs per agent; a partial unique index enforces it. A waiting turn holds later queued turns until it is retried or cancelled. Cancelling a turn marks it `cancelled` and the channel delivers `cancel`. When the credential stops authorizing the sandbox, the channel delivers `revoked` and closes; stopping the sandbox is the hard stop. While a drain request exists the channel delivers `drain`. The agent finishes its active turn and replies `drained`, then the orchestrator records the acknowledgement and answers `revoked`. A turn left running by an attempt whose session is no longer valid becomes `waiting` with `execution_interrupted`.

The agent keeps unacknowledged events in memory and resends them after reconnecting. It journals each tool call before running it and each result before reporting it. After a crash it reports a journaled result, or an `uncertain` result for a call that never finished, and never runs the command again. A tool call with no definite result after its run ended is shown as uncertain and blocks retry of that turn.

Each channel polls PostgreSQL once a second for new turns, cancellation, drain and revocation.

### Explicit lifecycle controller configuration

Set `CUBEBOT_CONTROLLER_CONFIG` to a local JSON file and configure `CUBEBOT_DATABASE_URL`. Supply `CUBEBOT_POD_NAMESPACE`, `CUBEBOT_POD_NAME` and `CUBEBOT_POD_UID` from the orchestrator Pod's Downward API metadata. The observed Pod must have one regular container named `orchestrator`, RestartAlways, a dedicated service account and no ephemeral containers. The ownership proof covers that container; init containers must not run independent orchestrator instances.

```json
{
  "connection": { "kind": "in_cluster" },
  "sandbox_image": "registry.example/cubebot-sandbox@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
  "backend": "kvm",
  "runtime_service_account": "cubebot-runtime",
  "orchestrator_origin": "https://cubebot.example",
  "network": { "dns": ["10.96.0.10"] }
}
```

`network` is required; supply one to three resolver IPs. Pods use `dnsPolicy: None` with exactly those resolvers, so use fully qualified hostnames for cluster services. Egress control is pod-level NetworkPolicy, which governs exactly one agent. `backend` is `kvm` or `process`; `kvm` adds a `devices.kubevirt.io/kvm` resource limit so the pod gets `/dev/kvm` from a device plugin, and `process` is development-only.

Replace the example image digest with the built sandbox image. For an explicitly selected kubeconfig, replace `connection` with `{"kind":"kubeconfig","path":"/trusted/kubeconfig","context":"cubebot"}`. The service never discovers the default context. Kubernetes transport requires certificate-verified HTTPS, with literal loopback HTTP allowed for local controlled workflows. The orchestrator origin supplied to agents requires HTTPS.

For an orchestrator endpoint signed by a private CA, set optional `control_ca_config_map` to an administrator-managed ConfigMap in the installation namespace containing a `ca.crt` PEM bundle. The compiler mounts that key read-only in the sandbox container and sets `CUBEBOT_CONTROL_CA_FILE`. Public roots remain trusted; certificate and hostname checks remain enabled. A missing key blocks Pod startup; an unreadable, empty, malformed or oversized bundle blocks runtime startup. The maximum bundle size is 64 KiB. Trust is loaded at runtime process startup, so replace Pods to apply CA changes. The ConfigMap contains public CA certificates, never a CA private key.

The controller scans durable agent intent in keyset batches of 32 and waits five seconds after each full scan. It checks process ownership before each reconciliation and bounds each attempt to sixty seconds. HTTP handling proceeds concurrently. Shutdown stops admission, settles handlers and the current reconciliation, then releases ownership. Ownership-check failure stops both loops and retains the claim. Agent drain and physical termination rules still control whether a replacement Pod is allowed.

Without controller configuration, database-only development mode holds the same exclusion lock and a durable development claim. A clean stop permits later controller activation. An unclean development exit retains the claim because there is no Kubernetes container proof for that local process; automatic takeover is deliberately unavailable. Once Kubernetes ownership has been established, omitting controller configuration is refused. Liveness-only mode without a database remains available.

### Authentication resource limits

The orchestrator permits at most four concurrent Argon2 computations across password hashing and verification. Additional work receives HTTP 429 with `Retry-After: 1`; it does not enter an unbounded hashing queue. The permit stays with the blocking computation if its HTTP caller disconnects.

Each active account has a persisted budget of ten password-verification attempts per 60-second window. Login reserves against that account; password-bearing command replay reserves against the command's actor. Successful and failed attempts both count, including a reservation followed by a busy hash worker. Invitation replay and login share the member's budget, so replay is not an alternate unlimited password-checking path. Exhaustion returns the generic unauthorized response until the window resets. Restart does not clear the budget, and existing sessions remain usable.

### Teams and human permissions

Administrators manage local access teams through `GET/POST /api/identity/teams`, `POST /api/identity/teams/membership` and `POST /api/identity/teams/delete`. Team creation accepts `request_id` and `name`; membership changes accept `request_id`, `team`, `account_id`, `expected_revision` and `present`. Deletion accepts `request_id`, `team` and `expected_revision`. Listing returns at most 100 records; use the last team's ID as `?after=<id>` for the next page. Deleting a team disables its access while preserving its records. Suspended accounts can be removed from teams, but cannot be added.

`POST /api/identity/access/grants` accepts `request_id`, a `recipient` object with `kind` of `user` or `team` and its `id`, a `scope` object with `kind` of `agent`, `room` or `conversation` and its `id`, and a named `capability`. Supported capabilities are `agent_manage`, `conversation_read`, `conversation_send`, `memory_read`, `memory_edit`, `workspace_read`, `workspace_write`, `browser_view`, `browser_control` and `room_manage`. Conversation capabilities accept room/conversation scope; room management accepts room scope; the remaining capabilities accept agent scope. These are exact grants with no implied read/edit/control hierarchy. `POST /api/identity/access/revoke` accepts `request_id`, grant `id` and `expected_revision`.

`POST /api/identity/access/explain` accepts `scope` and `capability` and explains the authenticated user's current access, including direct and team grant sources. Administrators receive these defined permissions through their installation role. Ordinary membership grants no private-resource access, and an agent manager gains no unrelated content or grant-administration authority. Team/grant management is administrator-only initially. Agent and text-conversation handlers check resource existence and content access.

Team/grant, suspension and revocation mutation responses include an ID and revision. Their `request_id` is scoped to the authenticated actor and command kind: retries return the recorded receipt, while changed request content returns a conflict. `GET /api/identity/commands/<kind>/<request_id>` reads the caller's receipt, with current administrator authorization. Administrator command kinds are `account.create`, `account.suspend`, `invitation.create`, `invitation.revoke`, `team.create`, `team.membership`, `team.delete`, `access.grant`, `access.revoke` and `oidc.revoke`. These receipts survive restart. Password-bearing commands keep a separate Argon2 verifier to reject changed-password retries without storing a fast password digest. Password verification runs outside database transactions. Administrator receipt retrieval never returns that verifier.

`GET /api/identity/access/events?after=<sequence>` returns up to 100 administrator-visible mutation records, including the actor, affected resource, revision and non-secret request. Publication is serialized through transaction commit so a cursor cannot skip a lower event that commits late. Account/invitation events include only non-secret metadata; passwords, invitation tokens and password verifiers are excluded from their audit payloads.

`GET /api/health/ready` checks the configured database and exists only in database-enabled mode. It does not attest to agent, Kubernetes or provider readiness.

```sh
just integration
```

This builds the orchestrator/operator binaries, starts an isolated disposable PostgreSQL 18 container with a generated password and temporary host-backed data directory, runs real database/HTTP tests, signed local OIDC issuer workflows and process restarts, then removes that container and its test data. When `just db-up` is running it reuses that server instead: it creates databases named `cubebot_it_<run>*` and drops them afterwards, and never touches the application database. Migrations run once into a `cubebot_template_<hash>` database, which persists on a reused server; each test copies it with `CREATE DATABASE ... TEMPLATE`. Tests run in parallel, one process per test, because password hashing limits concurrent work per process. Ordinary `cargo test` explicitly reports the database cases as ignored; `just integration` runs them and fails if Docker or PostgreSQL is unavailable.

## Builds and containers

```sh
cargo build --release --workspace --locked
bun run build
bun run --cwd apps/web start
```

The frontend server listens on port 3000 by default. Set `HOST=127.0.0.1` for local-only production preview. Its [documented Bun deployment adapter](https://tanstack.com/start/latest/docs/framework/react/guide/hosting#bun) is Nitro, pinned to `3.0.260903-beta`. The current build emits upstream bundler warnings about React Router's `use client` directives; the local browser check found no console errors or warnings.

```sh
docker build -t cubebot-orchestrator:dev -f apps/orchestrator/Dockerfile .
docker build -t cubebot-web:dev -f apps/web/Dockerfile .
```

The sandbox image (`apps/sandbox/Dockerfile`) runs as UID 10001 and contains `cubebot-sandbox` and `cubebot-agent`.

### Docker sandboxes without a cluster

Set `CUBEBOT_DOCKER_CONFIG` to a JSON file instead of `CUBEBOT_CONTROLLER_CONFIG`:

```json
{
  "image": "cubebot-sandbox:dev",
  "backend": "process",
  "orchestrator_origin": "http://host.docker.internal:8080"
}
```

Optional `terminal_enabled` works as in the controller configuration. The controller runs under development ownership and serves the runtime API. Each running agent gets one retained volume (`cubebot-workspace-<agent>`) and one container per epoch (`cubebot-agent-<agent>-<epoch>`) through the local `docker` CLI. Containers are recorded as pod attempts in namespace `docker`, so sessions, drains, epochs and restart generations behave as on Kubernetes: a crashed or stale container is stopped and its final state retained before a new epoch starts, and a running agent is drained before it is stopped.

Each container gets a fresh bootstrap token in `CUBEBOT_BOOTSTRAP_TOKEN`; the orchestrator stores only its SHA-256 digest. The token logs in only while that container's attempt is bound and observed running. The agent accepts HTTPS, or HTTP only to loopback, so a loopback `orchestrator_origin` runs the container with host networking (on Docker Desktop, enable host networking).
