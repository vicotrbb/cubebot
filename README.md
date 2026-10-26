# Cubebot

A Kubernetes-native home for persistent agents, shared rooms, memories, and skills.

**Status: first persistence and identity slice in progress.** The Rust orchestrator supports PostgreSQL migrations, operator-only administrator bootstrap, built-in account login, expiring sessions, logout, administrator account creation and suspension with durable request receipts, and login-bound invitations with one-time redemption, expiry and revocation, local human teams and exact scoped human permissions. Optional OIDC code/PKCE sign-in and verified account linking share the same platform sessions. The TanStack Start frontend still serves a starting page. Agent execution, rooms, memory, integrations, PWA installation and Kubernetes reconciliation are not implemented yet.

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
| `modules/providers` | Shared provider adapter foundation; no providers implemented yet      |
| `tooling/quality`   | Complexity gate, regression tests, and Python development environment |
| `tests/system`      | Smoke checks spanning built application processes                     |
| `deploy`            | Installation packaging; Dockerfiles live with their apps              |
| `docs/adr`          | Accepted product and architecture decisions                           |

See [repository layout](docs/development/repository-layout.md) for the complete module map and dependency rules. The agent package builds `cubebot-agent` and `cubebot-tool-runner`; both explicitly exit with an unimplemented message until their execution behavior exists. The tool runner belongs to the agent app and will run in its separate tool container.

The backend remains a modular monolith. PostgreSQL uses SQLx with embedded versioned migrations. Kubernetes, provider and tool dependencies will be added with their first working feature.

## Quality rules

**Cyclomatic complexity must be <= 15 per authored function, including tests and tooling.** This is a hard maximum, not a codebase average. `just complexity` scans the repository, including newly added directories. Rust `match` arms, anonymous functions, and the inclusive 15/16 boundary have regression coverage.

See [the quality policy](docs/development/quality.md) for exact tools, scope, exclusions, and limitations. `just format` applies formatting. GitHub Actions runs the same `just check` and `just smoke` commands and separately builds both containers. Hosting the workflow does not enable branch protection; configure required checks when a remote repository is created.

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

`POST /api/identity/oidc/start` returns an authorization URL and sets a ten-minute HttpOnly browser flow cookie. Follow the URL, then return through the callback. Cubebot validates state, browser binding, PKCE, nonce, signature, issuer, audience, expiry and any access-token hash before issuing its one-day platform session cookie and redirecting to `/`. The frontend sign-in screen remains pending. HTTPS session and flow cookies use `__Host-` names, `Secure`, `Path=/` and `SameSite=Lax`; loopback HTTP uses development cookie names. Unsafe cookie-authenticated requests require an exact matching `Origin`; bearer API clients keep their existing authentication path.

New membership requires either an exact admitted subject or an explicitly allowed domain on a verified email claim. Both lists default empty. Userinfo may supply a missing email, with its subject checked against the ID token. Admission creates an ordinary member; provider role/group claims confer no administrator status or local team membership. Existing active bindings can sign in without repeating first-login admission. Accounts are bound by trusted issuer and subject, never merged by matching email.

`POST /api/identity/oidc/link` starts explicit linking from a platform session created within ten minutes. That session must still be valid at callback. Each account has at most one active subject per issuer. Administrators inspect bindings with `GET /api/identity/oidc/bindings` (100 records, `?after=<id>` cursor), and revoke with `POST /api/identity/oidc/revoke`, supplying `id`, `expected_revision` and `request_id`. Revocation retains the binding as a tombstone and removes the account's OIDC-derived sessions, while built-in sessions remain independent. Suspension also prevents automatic re-admission. Re-admitting a revoked subject is deliberately unavailable in this slice.

Changing the non-secret OIDC configuration or removing it advances a persisted policy revision, invalidating pending flows and OIDC sessions. Rotating the client secret alone does not invalidate platform sessions. Built-in credentials remain available for installation recovery. Discovery/JWKS are refreshed during callback; provider requests have deadlines, reject redirects and limit response bodies to one MiB. Four concurrent OIDC operations and 1,000 pending unexpired flows bound basic resource use. This slice is tested with a controlled signed issuer; no external identity-provider compatibility or production security qualification is claimed.

### Authentication resource limits

The orchestrator permits at most four concurrent Argon2 computations across password hashing and verification. Additional work receives HTTP 429 with `Retry-After: 1`; it does not enter an unbounded hashing queue. The permit stays with the blocking computation if its HTTP caller disconnects.

Each active account has a persisted budget of ten password-verification attempts per 60-second window. Login reserves against that account; password-bearing command replay reserves against the command's actor. Successful and failed attempts both count, including a reservation followed by a busy hash worker. Invitation replay and login share the member's budget, so replay is not an alternate unlimited password-checking path. Exhaustion returns the generic unauthorized response until the window resets. Restart does not clear the budget, and existing sessions remain usable. These basic controls still require broader operational load/security qualification.

### Teams and human permissions

Administrators manage local access teams through `GET/POST /api/identity/teams`, `POST /api/identity/teams/membership` and `POST /api/identity/teams/delete`. Team creation accepts `request_id` and `name`; membership changes accept `request_id`, `team`, `account_id`, `expected_revision` and `present`. Deletion accepts `request_id`, `team` and `expected_revision`. Listing returns at most 100 records; use the last team's ID as `?after=<id>` for the next page. Deleting a team disables its access while preserving its records. Suspended accounts can be removed from teams, but cannot be added.

`POST /api/identity/access/grants` accepts `request_id`, a `recipient` object with `kind` of `user` or `team` and its `id`, a `scope` object with `kind` of `agent`, `room` or `conversation` and its `id`, and a named `capability`. Supported capabilities are `agent_manage`, `conversation_read`, `conversation_send`, `memory_read`, `memory_edit`, `workspace_read`, `workspace_write`, `browser_view`, `browser_control` and `room_manage`. Conversation capabilities accept room/conversation scope; room management accepts room scope; the remaining capabilities accept agent scope. These are exact grants with no implied read/edit/control hierarchy. `POST /api/identity/access/revoke` accepts `request_id`, grant `id` and `expected_revision`.

`POST /api/identity/access/explain` accepts `scope` and `capability` and explains the authenticated user's current access, including direct and team grant sources. Administrators receive these defined permissions through their installation role. Ordinary membership grants no private-resource access, and an agent manager gains no unrelated content or grant-administration authority. Team/grant management is administrator-only initially. Resource existence and actual content access must also be checked by the owning agent/room/conversation handlers, which are not implemented yet. This identity slice does not demonstrate those future content workflows.

Team/grant, suspension and revocation mutation responses include an ID and revision. Their `request_id` is scoped to the authenticated actor and command kind: retries return the recorded receipt, while changed request content returns a conflict. `GET /api/identity/commands/<kind>/<request_id>` reads the caller's receipt, with current administrator authorization. Administrator command kinds are `account.create`, `account.suspend`, `invitation.create`, `invitation.revoke`, `team.create`, `team.membership`, `team.delete`, `access.grant`, `access.revoke` and `oidc.revoke`. These receipts survive restart. Password-bearing commands keep a separate Argon2 verifier to reject changed-password retries without storing a fast password digest. Password verification runs outside database transactions. Administrator receipt retrieval never returns that verifier.

`GET /api/identity/access/events?after=<sequence>` returns up to 100 administrator-visible mutation records, including the actor, affected resource, revision and non-secret request. Publication is serialized through transaction commit so a cursor cannot skip a lower event that commits late. Account/invitation events include only non-secret metadata; passwords, invitation tokens and password verifiers are excluded from their audit payloads. Event streaming and domain-wide notifications remain future work.

`GET /api/health/ready` checks the configured database and exists only in database-enabled mode. It does not attest to agent, Kubernetes or provider readiness.

```sh
just integration
```

This builds the orchestrator/operator binaries, starts an isolated disposable PostgreSQL 18 container with a generated password, runs real database/HTTP tests, signed local OIDC issuer workflows and process restarts, then removes that container and its test data. It never uses the database in `.env` or another application's database. Ordinary `cargo test` explicitly reports the database cases as ignored; `just integration` runs them and fails if Docker or PostgreSQL is unavailable.

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

Container recipes run as non-root users. They are development build foundations, not a qualified installation. Helm packaging, agent pod construction, network enforcement, credential isolation, and fencing belong to subsequent implementation milestones.

Container verification was retried after the layout change on 2026-10-25. Both builds remained blocked by Docker's `no space left on device` error: the orchestrator during dependency compilation and the frontend while materializing its base image. No existing Docker data was pruned. See [layout verification](docs/development/layout-verification.md) for the successful local and fresh-copy checks; image builds remain unqualified.

## Design and next milestone

- [Manifesto](MANIFESTO.md)
- [Architecture](ARCHITECTURE.md)
- [Accepted ADRs](docs/adr/README.md)
- [Software specification](docs/design/software-specification.md)
- [Qualification plan](docs/design/qualification.md)
- [Implementation milestones](docs/superpowers/plans/2026-10-25-cubebot.md)
- [Implementation evidence](docs/development/implementation-progress.md)

Next, implement a complete path from creating an agent and recording its assignment to native execution, streamed results, and restart-safe state. The scaffold does not resolve the security and recovery qualification gates described in those documents.
