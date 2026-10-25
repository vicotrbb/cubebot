# ADR 0013: Autonomous tool use within granted permissions

Status: Accepted

Date: 2026-10-23

The native Rust engine executes tools under Cubebot's permission model. Tool availability and permission to invoke it must have explicit semantics, including during the bounded disconnection behavior in [ADR 0012](0012-agent-disconnection-behavior.md).

Decision: Agents act autonomously within granted permissions. The user selected this model on 2026-10-23. Once an authorized manager grants a capability, the agent may use it without human approval for each invocation. The initial design does not include a per-action approval queue or agent-specific approval mode.

Usage: An agent granted permission to send email may send email during its assigned work without requesting approval for every message. An agent with read-only access cannot obtain sending permission merely by requesting it in a prompt or choosing another tool. The grant's scope determines which operations and resources are permitted.

Permission checks still apply at execution. Grants do not bypass resource scope, credential validity, execution budgets, cancellation rules, or installation restrictions. The authority to grant capabilities is separate from the capability itself; agents cannot expand their own permissions unless a future explicit decision grants that administrative authority.

Ownership: The authorization module owns grant semantics and evaluation. Tool execution enforces the resulting constraints, including when tools can invoke other programs or access the network. Prompt instructions alone are insufficient enforcement. The detailed controls for terminals, browsers, and connectors remain part of the tool design.

Rationale: This model supports unattended agent work and places the human decision at capability assignment rather than every tool invocation.

Alternatives considered: Mandatory review for designated sensitive actions would interrupt those workflows for human approval. Configurable approval policies per agent would support supervised and autonomous modes but add policy and approval lifecycle complexity. The user selected autonomous operation within grants.

Consequences: Managers must understand the authority a grant conveys, particularly for tools that can modify external systems. Grant descriptions and operational records must make that authority and its use inspectable. Cubebot must report denied or expired access rather than silently elevate it. External systems may still impose their own authentication or approval requirements.

During disconnection, existing grants may be used only within the valid offline authorization and execution limits defined by ADR 0012. A missing grant, expired authority, or unavailable required state causes the affected operation to wait or fail according to its contract. Reconnection is required to obtain new central authorization. Immediate revocation cannot be assumed visible while disconnected.

Related decisions: [ADR 0033](0033-separate-runtime-and-tool-containers.md) places agent-directed terminal and browser workloads in a tool container, separate from the credential-holding runtime within the same pod. [ADR 0034](0034-agent-software-installation.md) permits software installation within grants without per-package approval, with enforcement based on actual execution, filesystem, and network authority.

Network policy: [ADR 0035](0035-tool-network-access-defaults.md) grants public internet reachability by default and requires explicit grants for private destinations, subject to stricter installation restrictions. Application credentials and operation permissions remain separately enforced.

External integrations: [ADR 0040](0040-native-core-tools-and-mcp-integrations.md) keeps core tools native and uses MCP as the default external extension path. MCP tools follow the same grant and autonomous invocation policy; connecting a server does not authorize every tool it exposes.

Integration grant policy: [ADR 0044](0044-tool-allowlists-and-resource-restrictions.md) selects named tool allowlists and supported resource restrictions, with honest capability reporting and enforcement across available execution paths.

Trigger autonomy: [ADR 0047](0047-autonomous-trigger-management.md) includes managing triggers for an agent's own work by default, within existing authority and installation limits, without a separate trigger-management grant.

Open details: Grant representation; manager delegation; integration-specific resource checks and credential scopes; terminal and browser enforcement; budget accounting; denial behavior; revocation propagation; and operational records.

Validation scenarios: Execute a granted external write without a human approval step; deny an ungranted operation; prevent a tool from escaping its enforced scope through another execution path; reject expired authority; and apply the bounded disconnection policy without creating a new approval workflow.
