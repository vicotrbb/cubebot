# ADR 0017: Agent creation as a granted capability

Status: Accepted

Date: 2026-10-23

Agents can delegate work to existing peers under [ADR 0016](0016-agent-messaging-and-delegation.md). Dynamic teams and swarms also need a decision on whether an agent may provision additional agents.

Decision: Agents may create other agents when explicitly granted that capability. The user selected this model on 2026-10-23. The creating agent requests creation through the orchestrator; it receives no direct Kubernetes or platform database authority.

The creation grant defines the permitted scope of provisioning and what authority the creator may assign to a new agent. Creation remains subject to installation quotas, resource capacity, and budget rules. Creating agents must not reset or bypass applicable aggregate limits. Detailed quota and budget accounting remain open.

New agents belong to the installation under [ADR 0002](0002-agent-ownership.md), with managers assigned through the authorized provisioning policy. The creator's identity is recorded as provenance, not as a separate ownership category. A new agent receives its own identity and memory ownership. Creation does not implicitly copy the creator's memories, workspace, credentials, or room memberships.

The creator may assign only permissions allowed by its creation grant. In particular, the ability to create an agent does not automatically authorize granting that new agent further creation rights, installation administration, arbitrary credentials, or access to additional rooms. These choices must be explicit in the grant and validated by the orchestrator.

Usage: An agent authorized to assemble a team requests an additional specialist with a purpose, configuration, and permitted resources. The orchestrator validates the request, records the new identity and provisioning intent, and manages execution under the existing lifecycle policy. The caller can then delegate work through the ordinary task interfaces when the new agent is eligible.

Ownership: Authorization validates creation and delegated permissions. Agent management owns identity and manager assignment. Scheduling and lifecycle modules own admission and pod provisioning. Retried creation requests must resolve to the same accepted creation rather than unintentionally provisioning duplicate agents.

Rationale: Separately granted creation supports dynamically assembled teams while retaining centralized control of resources and authority. Agents without that capability can still collaborate with existing peers.

Alternative considered: Restricting creation to humans would make provisioning predictable but prevent authorized workflows from independently adding specialists or workers. The user selected agent-initiated creation within grants.

Consequences: Provisioning, attribution, quota enforcement, and eventual retirement become part of agent-driven workflows. Creation is a centrally authorized operation and cannot occur while disconnected under [ADR 0012](0012-agent-disconnection-behavior.md). Automatic approval of a valid grant does not guarantee immediate compute availability.

Related decision: [ADR 0018](0018-created-agent-lifetime.md) permits persistent or task-bound lifetime according to the creation grant, defaulting agent-initiated creation to task-bound workers.

Related decision: [ADR 0019](0019-created-agent-knowledge.md) initializes new agents with fresh memory and an explicit briefing, without copying the creator's saved memory.

Open details: Manager assignment defaults; creation interface; model and resource configuration limits; room admission; creation depth; quota reservations; and cleanup after partial provisioning or parent cancellation.

Validation scenarios: Authorized creation succeeds; an agent without the grant is denied; excessive delegated permissions are rejected; duplicate submission does not duplicate identity; concurrent creation respects quotas; pod provisioning failure preserves an inspectable state; and creation during disconnection does not allocate an unauthorized agent.
