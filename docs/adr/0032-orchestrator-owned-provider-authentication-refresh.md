# ADR 0032: Orchestrator-owned provider authentication refresh

Status: Accepted

Date: 2026-10-23

Agent runtimes call providers directly using propagated authentication under ADR 0031. Multiple agents can share one connection, so renewable authentication needs a clear owner for refresh and authoritative updates.

Decision: The orchestrator alone owns provider authentication refresh. It refreshes supported connections and propagates updated authentication material to authorized agent runtimes. The user selected this model on 2026-10-23. Agents use valid supplied material for inference and do not independently refresh the shared connection.

Usage: Several agents use the same renewable provider connection. The orchestrator performs its refresh, records the new authentication revision, and distributes the applicable update. Each agent continues calling the provider directly. No per-agent login or inference proxy is introduced.

Ownership: The provider-connection module in the orchestrator owns refresh scheduling, coordination, persistence, and distribution. Provider adapters implement the supported exchange and failure classification. Agent adapters consume supplied authentication state and report failures or the need for an update; they do not write replacement authoritative credentials. This uses the existing orchestrator process under ADR 0010.

Refresh operations for the same underlying authentication state must be coordinated. Creating two Cubebot connection records for one credential must not be assumed to create independent refresh authority. Version checks prevent a late refresh response or stale agent report from replacing newer state. Exact external-account identification and serialization mechanics remain open.

Propagate the material required for inference while keeping refresh-only secrets under orchestrator control when the provider's credential format permits that separation. Whether a particular authentication format supports such separation requires qualification. Shared authentication semantics do not require copying the orchestrator's entire credential store into each agent.

During an orchestrator outage, current agent work can continue only while its supplied authentication and bounded execution authorization remain valid. If authentication becomes unusable, affected inference waits for the orchestrator to restore access. The agent does not bypass this policy by attempting local refresh or another model. This narrows availability according to credential lifetime without changing ADR 0012's finite continuation limits.

Refresh failure must distinguish a temporary provider problem from invalid authorization or required administrator reauthentication. An expired or rejected credential must not be reported as healthy. Static credentials are rotated through installation administration; they do not acquire a refresh operation merely to fit a common interface.

Implementation requirements: Preserve an identifiable committed authentication revision, propagate updates only to currently authorized recipients, and reconcile lagging agents on reconnect. Refresh can involve an external effect followed by local persistence; a crash between those steps cannot be assumed to roll back the provider's change. Recovery behavior and safe retries must be qualified per provider, with uncertain outcomes visible rather than blindly repeating exchanges.

Alternatives considered: Agent runtimes could receive refresh authority and synchronize results. That could extend independence for some providers but adds shared-state coordination and recovery across disconnected agents. The user selected one refresh owner.

Consequences: Agents remain independent for inference while supplied credentials are usable. Renewal depends on orchestrator availability. Refresh health and propagation failures need inspectable status, without exposing credential contents. Token lifetimes do not replace Cubebot's own execution authorization limits.

[ADR 0066](0066-encrypted-credentials-with-separate-installation-key.md) selects encrypted PostgreSQL records for authoritative credentials. External refresh revisions and storage-key versions remain distinct and must coordinate without losing newer authentication state.

Open details: Refresh lead time and retry policy; authentication revision schema; delivery acknowledgements; shared-credential identification; provider-specific uncertain-outcome recovery; reauthentication UX; and credential storage and deletion in agent runtimes.

Validation scenarios: Several agents receive one coordinated refresh update; agent runtimes never initiate refresh; stale reports cannot restore old credentials; a disconnected agent waits when its supplied authentication expires; a reconnecting agent receives the current authorized revision; revoked agents receive no further updates; static-key rotation works without a refresh exchange; and a crash after provider-side rotation exposes a recoverable or explicitly unresolved state.
