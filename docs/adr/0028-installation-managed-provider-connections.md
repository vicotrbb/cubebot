# ADR 0028: Installation-managed provider connections

Status: Accepted

Date: 2026-10-23

Cubebot agents belong to the installation and use model providers through the native Rust engine. Provider access needs an administrative owner and explicit rules for which agents may use each connection.

Decision: Support installation-managed provider connections only. The user selected this model on 2026-10-23. Installation administrators configure connections and grant agents access. Cubebot does not introduce a separate category of user-owned personal provider connections.

Access-policy amendment, 2026-10-25: [ADR 0081](0081-installation-wide-default-connection-access.md) makes enabled provider connections available to all eligible agents by default, with a per-connection option for specific grants. This replaces the original requirement for individually assigned access; installation-managed ownership remains accepted.

Usage: An administrator configures a provider connection and chooses all-agents access or specific grants. A manager selects from effectively authorized connections. New agents receive the all-agents baseline automatically; human users still require their applicable management permissions. A single-person installation follows the same model through its administrator account.

Installation-managed describes ownership of the Cubebot connection record and its administration. It does not transfer ownership of an external account, change subscription eligibility, or guarantee continued authorization. Supported API credentials, subscription authentication, and local-model endpoints can fit this ownership model; actual authentication paths still require qualification under ADR 0006. A connection need not have a secret when its supported endpoint does not require one.

The connection record is independent of the administrator account that created it. Removing that administrator must not silently delete the connection or expand anyone else's authority. Continued use still depends on valid external authorization, an authorized administrator, agent grants, and installation policy. External revocation or required reauthentication remains a real loss of access even when the Cubebot record survives.

Ownership: Provider configuration owns connection metadata and lifecycle. Authorization owns agent-use grants and administration rights. Provider adapters implement supported authentication behavior and inference. Under [ADR 0031](0031-direct-inference-and-propagated-authentication.md), the orchestrator propagates applicable authentication material to authorized agent runtimes, which call providers directly. Runtime credential access does not imply user, model, or tool permission to inspect or export the secret. Exact storage, rotation mechanics, and runtime/tool isolation remain separate design choices. ADR 0032 assigns refresh ownership to the orchestrator.

Connection-wide provider limits must be accounted for across all agents using the same connection. Multiple records pointing to the same external account must not be assumed to have independent quotas. The accounting and external-account identity mechanisms remain open.

Disabled or revoked access must be enforced according to the credential and execution contracts. During disconnection, ADR 0012 still limits use to existing valid authority; this decision does not promise immediate offline revocation or offline credential refresh.

Alternatives considered: Supporting both installation and personal connections would permit separately owned user accounts with their own sharing and departure rules. The user selected one administrative ownership model for simpler management.

Consequences: Users cannot independently attach private provider connections outside installation administration. Organizations manage provider availability and spending centrally. Personal installations retain access to supported authentication methods through the same administrator workflow.

Scope: This decision covers model-provider connection ownership. It does not settle ownership of email, calendar, repository, browser, or other tool integrations. Model selection, failure behavior, and direct inference are covered by ADRs 0029, 0030, and 0031. The credential backend remains open.

Related decisions: [ADR 0029](0029-model-default-and-purpose-overrides.md) selects a default model per agent with optional overrides for extraction, dreaming, and compaction, using granted connections. [ADR 0030](0030-retry-selected-model-without-fallback.md) requires bounded retries and visible waiting instead of automatically switching connections or models.

Refresh ownership: [ADR 0032](0032-orchestrator-owned-provider-authentication-refresh.md) assigns refresh exclusively to the orchestrator, which distributes updated authentication to authorized agents.

Open details: Connection schema; secret storage and delivery; supported authentication methods; rotation and refresh mechanics; revocation propagation; shared-account limit tracking; usage accounting; and provider qualification.

Validation scenarios: Use a default connection from existing and new agents; grant selected agents access to a restricted connection; deny agents without effective access; prevent secret disclosure through use authority; retain the connection record after its creator leaves; handle external revocation without granting replacement access; and apply aggregate limits when several agents share a connection.
