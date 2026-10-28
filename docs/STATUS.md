# Status

Updated 2026-10-27. This page lists what runs and what has not been qualified. [ARCHITECTURE](../ARCHITECTURE.md) describes the intended system; [the reference](development/reference.md) documents current configuration and APIs.

## What runs

- Built-in sign-in/out, accounts, invitations, teams, scoped grants and optional OpenID Connect.
- Authorized agent creation, editing and lifecycle intent, with Kubernetes reconciliation when a controller is configured.
- Encrypted OpenAI API-key/model configuration and one permanent text conversation per agent, served by the native Rust runtime through the website.
- A terminal implementation, disabled by default. See [terminal execution](development/bounded-terminal.md).

Evidence: [text chat](development/text-chat-mvp.md), [container isolation](development/container-isolation.md), [cluster admission](development/cluster-admission.md), [cluster startup](development/cluster-startup.md), [layout verification](development/layout-verification.md).

## Code behind the accepted design

[ADR 0083](adr/0083-agent-centric-execution-in-a-sandbox-boundary.md) replaces per-turn grants, invocation leases and the two-container pod with one VM-class sandbox per agent and coarse agent authority. The code still implements the earlier model: finite per-turn inference grants, renewable terminal leases, a separate tool container, a privileged network initializer and UID-aware kernel egress filtering. No VM RuntimeClass is selected or tested.

## Not yet qualified

Agent system

- Rooms, delegation, memory, skills, browser, MCP, integrations, triggers, notifications, PWA/push and token streaming are not implemented. Responses commit as complete messages.
- Room content workflows and event streaming are not implemented.
- Typed invocation/result reconciliation, semantic reconciliation of recovered outcomes and journal crash-window behavior on CSI are unfinished.

Kubernetes and isolation

- Real-cluster qualification of lifecycle, fencing and active-work drain is pending. Kubernetes boundary tests are local API-contract tests, not proof of physical fencing.
- Mount, credential, network and fencing qualification remain installation prerequisites. CNI/CSI, DNS, grants, browsers and arbitrary-tool isolation (G1–G3) are pending.
- The container-isolation results favor UID-aware kernel filtering; production network enforcement and full credential isolation are not qualified.
- Connected-cluster operation with a private control CA is unqualified; only local TLS tests exist.
- Database-only development mode is not production recovery qualification.
- `just cluster-admission` creates no running agent or workspace; CSI, CNI and full lifecycle qualification are pending.

Identity and security

- OIDC is tested only with a controlled signed issuer. No external identity-provider compatibility or production security qualification is claimed.
- Authentication rate and concurrency limits need broader load and security qualification.
- Re-admitting a revoked OIDC subject is unavailable. Invitation token delivery is manual. There is no remote authentication-management UI.

Packaging and operations

- Helm packaging does not exist yet.
- Production PostgreSQL image digests and pgvector are unqualified; the development image follows PostgreSQL 18.
- The frontend's Nitro Bun adapter is a prerelease dependency. Bun SSR and static assets are smoke-tested; production hosting is unqualified.
- Container image builds are unqualified. The 2026-10-25 retry was blocked by Docker running out of disk space.
- Security, fencing and scale (ADR 0061: 100 registered, 25 working agents) require the [qualification plan](design/qualification.md).
