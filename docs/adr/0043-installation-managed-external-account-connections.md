# ADR 0043: Installation-managed external account connections

Status: Accepted

Date: 2026-10-23

Administrators configure integration definitions under ADR 0042. The accounts and credentials used through those definitions need an ownership model independent of the external service's own account ownership.

Decision: External account connections are installation-managed only. Administrators manage connection records and determine which agents may use them. The user selected this model on 2026-10-23. Cubebot does not introduce a separate user-owned connection category for email, calendars, repositories, or other external integrations. Reusable website account access follows the same administrative model.

Access-policy amendment, 2026-10-25: [ADR 0081](0081-installation-wide-default-connection-access.md) makes enabled external account and reusable website connections usable by all eligible agents by default, with a per-connection specific-grants mode. Installation ownership and administrator configuration remain unchanged.

Usage: An administrator connects a mailbox through an approved integration and leaves all-agents access enabled, or selects specific grants for restricted access. A manager may assign a restricted connection only within delegated authority. The external account holder may still need to authenticate or consent, but the resulting Cubebot connection remains an installation resource rather than a personal connection owned by the assisting user.

Installation management does not transfer ownership of the external account, bypass its consent requirements, or guarantee authorization after revocation. It also does not expose the connection's content or raw credentials to every installation user. Existing content access, agent-use grants, and external account permissions remain independently applicable.

Ownership: Integration configuration owns connection identity, administrative configuration, and lifecycle. Authorization owns assignment and use grants. Integration adapters implement supported authentication and account-specific operations. The browser module uses authorized account state under ADR 0038. Exact secret storage, refresh behavior, and execution placement remain open; model-provider refresh ownership under ADR 0032 does not automatically define every website or integration's authentication flow.

Connection identity must distinguish the integration definition from the external account used through it. Several accounts may use one approved integration without becoming interchangeable. Invocation records must identify the selected connection, without logging its secrets. A tool must not silently select a different account when the configured one is unavailable.

A connection record is independent of the administrator or assisting user who created it. User departure does not itself transfer or broaden its grants. Continued execution requires valid external authorization and current installation policy. Disconnection, revocation, and removal must be recorded explicitly, with treatment of already-running work governed by execution and credential recovery contracts.

Browser takeover under ADR 0039 may assist with authentication, but control of a browser session is not permission to register a new account connection or broaden reusable account grants. The onboarding flow must distinguish an administrator-authorized connection setup from ordinary session assistance. A login alone must not silently publish credentials to additional agents or tasks.

Alternatives considered: Support both installation-managed and user-owned connections through administrator-approved integrations. That would support user-controlled account sharing but add another ownership, consent, and departure model. The user selected one administrative model, consistent with model-provider connection ownership in ADR 0028.

Consequences: External account onboarding and connection administration require administrator involvement. Ordinary users cannot independently create private connection records. Personal installations use the same workflow through their administrator account. External consent and credential validity still constrain what the installation can do.

Grant policy: [ADR 0044](0044-tool-allowlists-and-resource-restrictions.md) selects named tool allowlists plus Cubebot resource restrictions where enforceable, alongside external credential scopes.

Open details: Connection and account identity schema; assignment delegation; grant representation and enforcement mappings; consent and browser onboarding UX; secret storage and delivery; authentication renewal; revocation propagation; audit retention; and in-flight disconnect behavior.

Validation scenarios: Administrators connect and assign an external account; delegated managers stay within assignment authority; ordinary users cannot create personal connection records; two accounts on one integration remain distinguishable; browser-assisted login does not broaden grants; creator departure preserves the record without extending authority; and external revocation makes affected access unavailable honestly.
