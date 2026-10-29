# Status

Updated 2026-10-29. This page lists what runs and what has not been qualified. [ARCHITECTURE](../ARCHITECTURE.md) describes the intended system; [the reference](development/reference.md) documents current configuration and APIs.

## What runs

- Built-in sign-in/out, accounts, invitations, teams, scoped grants and optional OpenID Connect.
- Authorized agent creation, editing and lifecycle intent, with Kubernetes reconciliation when a controller is configured.
- Encrypted OpenAI API-key/model configuration and one permanent text conversation per agent, served by the native Rust runtime through the website.
- Built-in tools (`bash`, file, `glob`, `grep`) and an MCP client for stdio and Streamable HTTP servers, disabled by default, run by the agent inside its sandbox as a configured tool user, with parallel calls. See [tool execution](development/bounded-terminal.md). MCP tools have no per-tool grants yet.
- `cubebot-sandbox run`, which boots each agent in a KVM guest, launched as one container per agent by the Kubernetes controller or, without a cluster, through Docker. On an x86_64 Linux host with `/dev/kvm` it also runs directly, inside a network namespace of its own. The development `process` backend was removed on 2026-10-29: there is no way to run an agent outside a guest, and the sandbox image does not build without the guest.
- Operator revocation of an agent's runtime credential with `cubebot-admin revoke-runtime`.

Evidence: [text chat](development/text-chat-mvp.md), [cluster admission](development/cluster-admission.md), [cluster startup](development/cluster-startup.md), [layout verification](development/layout-verification.md).

## Code behind the accepted design

[ADR 0083](adr/0083-agent-centric-execution-in-a-sandbox-boundary.md) replaces per-turn grants and invocation leases with coarse agent authority, and [ADR 0084](adr/0084-in-house-vmm-as-the-agent-sandbox.md) makes Cubebot's own KVM VMM the sandbox. The tool container, network initializer and kernel egress baseline are gone; the agent runs its tools itself. Runtime control is one authenticated WebSocket per sandbox, with an agent credential that lasts for the sandbox incarnation; per-turn grants, terminal leases and journal custody are gone. The sandbox boots a guest from a read-only root disk with a workspace disk, virtio-net on a tap redirected to the pod interface, and a vsock control channel that carries the agent's configuration in and liveness out (see the [sandbox](../apps/sandbox/README.md)). `just guest-boot` checks in CI, against a stand-in orchestrator in a network namespace, that the guest agent reaches it from the pod address, reports liveness and keeps its journal across a restart. The orchestrator launches sandboxes as root with only `NET_ADMIN`, `/dev/kvm` and `/dev/net/tun`, which needs a privileged Pod Security namespace, and the sandbox image always carries the guest; CI builds that image only in its guest job. `just cluster-sandbox` covers ADR 0083's sandbox scenarios in kind, with a generic device plugin and a fixture NetworkPolicy. On 2026-10-29 it passed on an x86_64 AMD host with `/dev/kvm`, kind running inside a privileged pod: an agent ran turns against OpenAI `gpt-4.1-mini` inside KVM guests, and all six scenarios passed. Those are: the agent runs in the guest, not its pod; the guest reaches its gateway but not another sandbox, PostgreSQL or the Kubernetes API, the API probe included, under kindnet; a running command survives an orchestrator SIGKILL and commits once; stopping the agent ends its VM; a revoked credential loses work delivery and ledger writes; and a sandbox lost mid-task resumes from its journal without repeating its committed effect ([evidence](development/evidence/2026-10-29-cluster-sandbox.json); its `source` fingerprint `d2ea1ee5…` is the committed tree before this page and the evidence file were added, and its `base_commit` names the runner's own snapshot commit). That run's `just guest-boot` skipped only the bare-host refusal check, which cannot trip inside a pod. This is one kind run, not a qualified isolation boundary on a production CNI. The nightly workflow now runs `cluster-admission`, `cluster-startup`, `cluster-chat` and `cluster-sandbox` on a GitHub-hosted x86_64 runner with `/dev/kvm`, every one with guest-built images; it has not run there yet.

## Not yet qualified

Agent system

- Rooms, delegation, memory, skills, browser, integrations, triggers, notifications, PWA/push and token streaming are not implemented. Responses commit as complete messages.
- Room content workflows and event streaming are not implemented.
- Tool-journal crash-window behavior on CSI is unqualified. Each channel polls PostgreSQL once a second, and each open browser conversation stream (`GET /api/conversations/{id}/events`) twice a second; delivery latency and database load at scale are unmeasured.

Kubernetes and isolation

- Real-cluster qualification of lifecycle, fencing and active-work drain is pending. Kubernetes boundary tests are local API-contract tests, not proof of physical fencing.
- Mount, credential, network and fencing qualification remain installation prerequisites. CNI/CSI, DNS, grants, browsers and arbitrary-tool isolation (G1–G3) are pending.
- NetworkPolicy enforcement on guest traffic has passed only under kind's kindnet in one `just cluster-sandbox` run; it is not qualified on a production CNI.
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
