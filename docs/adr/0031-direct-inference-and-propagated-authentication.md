# ADR 0031: Direct provider inference with propagated authentication

Status: Partially superseded by ADR 0083

Date: 2026-10-23

Superseded in part by [ADR 0083](0083-agent-centric-execution-in-a-sandbox-boundary.md): provider authentication is propagated for the sandbox's lifetime, not as per-turn inference grants. Direct inference and orchestrator-owned refresh remain.

Provider connections are installation-managed under ADR 0028. Agent runtimes need a concrete inference path that supports the bounded continuation during orchestrator outages selected in ADR 0012.

Decision: Agent runtimes call model providers directly. The orchestrator propagates the authentication material and configuration needed for the connections granted to each agent. The user selected direct calls on 2026-10-23 and explicitly specified that agents have access to the same authentication mechanism as the orchestrator through this propagation.

This covers the supported authentication mechanism for the selected provider connection, rather than requiring every agent to be configured or logged in independently. It does not distribute all installation credentials to every agent. [ADR 0081](0081-installation-wide-default-connection-access.md) allows connection eligibility through an all-agents baseline or specific grants; propagation still supplies only the applicable material needed for selected authorized inference. Provider authentication remains distinct from the agent's identity when calling Cubebot, Kubernetes authority, and database access; agents still receive no platform database credentials under ADR 0009.

Usage: An administrator configures a provider connection and grants an agent access. The orchestrator authenticates that agent and supplies the applicable provider configuration and authentication material. The agent runtime constructs model requests, authenticates directly with the provider, consumes the response stream, and reports execution and usage records to the orchestrator. Model payloads and streams do not require an orchestrator inference proxy.

Ownership: The orchestrator owns connection administration, grant evaluation, and propagation of authorized authentication state. Provider adapters in agent runtimes implement the same supported authentication semantics used for orchestrator inference. The agent engine owns its local model and tool loop. Shared adapter code should keep those semantics consistent without introducing a second agent engine or separately administered personal connections.

Propagation must use an authenticated, confidential channel and identify the connection and authentication-state revision. Updates must not accidentally restore an older credential revision. Reconnecting agents reconcile their state with the orchestrator. The exact delivery mechanism, local secret persistence, encryption, bootstrap identity, and update acknowledgement protocol remain open.

Sharing an authentication mechanism does not imply shared refresh authority. [ADR 0032](0032-orchestrator-owned-provider-authentication-refresh.md) makes the orchestrator the sole refresh owner and authoritative distributor of updated authentication state. Agent runtimes do not refresh independently. Subscription support still requires qualification for this runtime and deployment; propagation alone does not establish provider permission or compatibility.

Trusted runtime access to provider authentication material is required by this decision. It does not imply placing secrets in model context, ordinary logs, shared workspace files, or the environment of agent-executed commands. [ADR 0033](0033-separate-runtime-and-tool-containers.md) selects distinct runtime and tool containers within the agent pod. The deployment and execution controls must enforce credential separation; implementation verification remains pending.

During disconnection, an agent may continue its existing turn using already-propagated, valid authentication within ADR 0012's finite execution authority and budgets. Missing or unusable authentication cannot be obtained from an unavailable orchestrator. No new grant is created by a locally cached credential. Provider failure follows ADR 0030's retry-and-wait policy without automatic connection or model switching.

Consequences: The orchestrator does not carry agent inference streams. Provider credentials exist in more execution environments, so isolation, rotation, and revocation need explicit design. Locally enforced authorization expiry is not the same as invalidating a copied external credential. Shared-account limits and aggregate spending still require coordination even though calls are distributed.

Alternative considered: Proxy all inference through the orchestrator, retaining provider credentials there. That would centralize request enforcement but make each new model request depend on orchestrator availability. The user selected direct inference and propagated authentication.

Scope: This decision governs agent inference and provider authentication. It does not grant arbitrary tools access to provider secrets, define tool-integration credential distribution, or move the room coordinator out of the orchestrator.

[ADR 0066](0066-encrypted-credentials-with-separate-installation-key.md) stores authoritative credentials encrypted in PostgreSQL with a separately held installation key. That key is not propagated to agents.

[ADR 0067](0067-shared-goal-budgets-with-optional-cost-limits.md) selects aggregate goal budgets for delegated work, with monetary limits optional and unset by default. Enabled limits require coordinated allocations and usage reconciliation despite direct provider calls.

Open details: Delivery and local storage; refresh coordination and recovery under ADR 0032; credential rotation and revocation; isolation enforcement under ADR 0033; bootstrap identity; distributed quota and budget enforcement; authentication-state versioning; and provider qualification.

Validation scenarios: A granted agent calls its provider directly; an ungranted connection is not propagated; an authentication update replaces the correct connection revision; reconnect rejects stale state; existing work continues during a short orchestrator outage within limits; expired authentication causes visible waiting; provider secrets stay out of model and tool outputs; and aggregate usage remains attributable across agents.
