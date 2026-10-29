# ADR 0083: Agent-centric execution in a sandbox boundary

Status: Accepted; sandbox mechanism amended by [ADR 0084](0084-in-house-vmm-as-the-agent-sandbox.md)

Date: 2026-10-27

The first implementation split one agent's work across the orchestrator and two containers. The orchestrator granted each turn, leased each tool invocation and renewed short authority windows. The runtime and tool runner shared a pod network, so credential separation needed UID-aware kernel filtering, a privileged network initializer and a root supervisor. Every new agent capability had to cross that machinery, and most engineering effort went into the control plane instead of the agent.

Decision: The agent owns its execution and runs inside one VM-class sandbox. The user selected this model on 2026-10-27.

- **The agent owns** its loop, context assembly and compaction, memory behavior, tools and recovery journal. It decides when to call a model, which tool to run, what to remember and what to retrieve.
- **The orchestrator is ledger and switchboard only.** As ledger it durably records what humans and recovery need: agent definitions, conversations, messages, results, memory records, usage and audit. As switchboard it authenticates agents and people, routes messages and work to agents, enforces content permissions, owns provider credential refresh and starts and stops sandboxes. It does not run turns, schedule lanes, authorize individual tool calls or decide memory policy.
- **One sandbox per agent.** Each running agent gets one VM from Cubebot's own VMM, `cubebot-sandbox` (amended by ADR 0084; this originally selected a VM-class Kubernetes RuntimeClass). The runtime, terminal, browser, local MCP servers and agent-installed software run together inside it. The VM edge is the trust boundary; isolation between the runtime and its own tools is not.
- **Coarse, long-lived authority.** A sandbox receives one agent-scoped credential for its lifetime, plus the provider credentials that agent is eligible for. There are no per-turn inference grants, invocation leases or renewal windows. Revocation means revoking that credential or stopping the sandbox.

Usage: A person sends a message. The orchestrator records it, checks the sender's content access and wakes or notifies the agent. The agent pulls the message, works through its own loop for as long as it needs, calls the provider directly and runs commands or a browser inside its sandbox. It writes results, memory changes and usage back to the ledger through idempotent operations. Stopping the agent stops the sandbox.

Rationale: Any credential inside the sandbox is reachable by code the agent runs there, so splitting authority per turn inside it bought little real security for a lot of protocol. Giving each agent only its own authority bounds the damage of a compromised sandbox to that agent. A VM boundary contains hostile code better than containers sharing a kernel and pod network, and pod-level NetworkPolicy now applies to exactly one agent.

What remains: Agents still never connect to PostgreSQL (ADR 0009). Provider inference stays direct (ADR 0031), and the orchestrator still owns refresh (ADR 0032). Content permissions, one permanent conversation (ADR 0082), installation-managed connections (ADR 0081) and explicit deletion (ADR 0079) are unchanged. Lanes (ADR 0005) become a scheduling rule inside the agent loop rather than an orchestrator admission rule.

Supersedes:

- ADR 0009: orchestrator modules owning agent memory policy and execution leases for reporting. The interface remains the only path to platform state.
- ADR 0012: finite per-turn authorization windows and the ban on starting queued work while disconnected. Disconnection now limits only what needs the switchboard.
- ADR 0031: per-turn, versioned inference grants. Provider material is propagated per sandbox lifetime.
- ADR 0033: separate runtime and tool containers in one pod and credential separation between them.

Relationship to ADR 0076: as amended by ADR 0084, an agent sandbox is a `cubebot-sandbox` KVM guest, not a RuntimeClass; since 2026-10-29 there is no unisolated development backend. Standard container runtimes remain usable for the orchestrator.

Consequences: The tool runner, network initializer, kernel egress filter, root supervisor and per-turn grant and lease protocol become removal candidates once the sandbox path is qualified. The installation must provide nodes that expose `/dev/kvm` to the sandbox pod (ADR 0084), which rules out some managed clusters without nested virtualization. Agents hold their provider keys for longer, so rotation and revocation must actually work. A compromised sandbox can spend that agent's provider budget and act as that agent until its credential is revoked. Budget limits (ADR 0067) are checked from ledger usage reports, not enforced per call.

Alternatives considered: Keep per-turn grants inside a VM sandbox, which keeps the protocol cost without a matching security gain. Run without a sandbox and rely on containers, which leaves hostile tool code sharing a kernel with the platform. Proxy all inference through the orchestrator, which ADR 0031 already rejected.

Open details: Credential lifetime and rotation; the revocation path when the orchestrator is unavailable; the ledger write contract and idempotency keys; which sandbox configurations are qualified; sandbox image and persistent workspace layout; how the agent loop schedules concurrent lanes; local development without a cluster.

Validation scenarios: A sandbox runs its agent in a `cubebot-sandbox` VM; a command in the sandbox cannot reach another agent's sandbox, the orchestrator database or the Kubernetes API; revoking the agent credential stops ledger writes and provider refresh; stopping the agent terminates the VM; a long task continues through an orchestrator restart and reconciles results without duplicates; a sandbox lost mid-task resumes from its journal and the ledger without repeating committed effects.
