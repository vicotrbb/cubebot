# Status

Updated 2026-10-28. This page lists what runs and what has not been qualified. [ARCHITECTURE](../ARCHITECTURE.md) describes the intended system; [the reference](development/reference.md) documents current configuration and APIs.

## What runs

- Built-in sign-in/out, accounts, invitations, teams, scoped grants and optional OpenID Connect.
- Authorized agent creation, editing and lifecycle intent, with Kubernetes reconciliation when a controller is configured.
- Encrypted OpenAI API-key/model configuration and one permanent text conversation per agent, served by the native Rust runtime through the website.
- Built-in tools (`bash`, file, `glob`, `grep`) and an MCP client for stdio and Streamable HTTP servers, disabled by default, run by the agent inside its sandbox as a configured tool user, with parallel calls. See [tool execution](development/bounded-terminal.md). MCP tools have no per-tool grants yet.
- `cubebot-sandbox` with the development `process` backend, launched as one container per agent by the Kubernetes controller or, without a cluster, through Docker.

Evidence: [text chat](development/text-chat-mvp.md), [cluster admission](development/cluster-admission.md), [cluster startup](development/cluster-startup.md), [layout verification](development/layout-verification.md).

## Code behind the accepted design

[ADR 0083](adr/0083-agent-centric-execution-in-a-sandbox-boundary.md) replaces per-turn grants and invocation leases with coarse agent authority, and [ADR 0084](adr/0084-in-house-vmm-as-the-agent-sandbox.md) makes Cubebot's own KVM VMM the sandbox. The tool container, network initializer and kernel egress baseline are gone; the agent runs its tools itself. Runtime control is one authenticated WebSocket per sandbox, with an agent credential that lasts for the sandbox incarnation; per-turn grants, terminal leases and journal custody are gone. The `kvm` backend only boots a guest kernel whose init is the agent, with a serial console and no virtio devices; `just guest-boot` checks that in CI. The guest agent cannot reach the orchestrator, so no working agent runs in a VM and no deployment has an isolation boundary between agent-run code and the pod.

## Not yet qualified

Agent system

- Rooms, delegation, memory, skills, browser, integrations, triggers, notifications, PWA/push and token streaming are not implemented. Responses commit as complete messages.
- Room content workflows and event streaming are not implemented.
- Tool-journal crash-window behavior on CSI is unqualified. Each channel polls PostgreSQL once a second; delivery latency and database load at scale are unmeasured.

Kubernetes and isolation

- Real-cluster qualification of lifecycle, fencing and active-work drain is pending. Kubernetes boundary tests are local API-contract tests, not proof of physical fencing.
- Mount, credential, network and fencing qualification remain installation prerequisites. CNI/CSI, DNS, grants, browsers and arbitrary-tool isolation (G1–G3) are pending.
- NetworkPolicy enforcement on sandbox pods, and on guest traffic once the `kvm` backend exists, is not qualified.
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
