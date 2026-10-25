# ADR 0050: Invitations and optional OpenID Connect admission

Status: Accepted

Date: 2026-10-23

Platform authentication establishes identity, but an installation also needs to control who may become a member. Personal installations need explicit admission; organizations may want eligible employees to join through their existing identity provider.

Decision: Use invitations or authorized account creation for built-in membership, with optional automatic admission through administrator-configured OpenID Connect rules. The user selected this model on 2026-10-23. Open public registration for built-in accounts is outside this selected model.

Usage: An administrator invites a colleague to join. Alternatively, an organization enables automatic admission for eligible users of its configured identity provider. At first successful qualifying login, Cubebot creates the member record with the configured baseline role. An authenticated identity that does not qualify and has no existing admitted membership receives no platform access.

Ownership: The orchestrator's identity module owns invitations, membership, and admission decisions. Installation administrators configure automatic admission and its baseline role. Authorization determines who may invite or create members and enforces the permissions of resulting memberships. Delegation of invitation authority remains to be specified.

Automatic admission is disabled unless configured. Rules apply to a specific trusted provider and validated identity attributes, using explicit eligibility criteria. The exact rule vocabulary remains open. A provider role or group claim does not independently grant Cubebot privileges; admission and any later role mapping follow installation configuration.

New members receive only their configured baseline authority. Membership does not automatically grant access to private agents, rooms, memories, or workspaces. [ADR 0051](0051-built-in-roles-and-scoped-resource-grants.md) selects built-in roles and scoped grants; the detailed matrix must make baseline access and any privileged assignment explicit. This decision does not select automatic administrator assignment or group synchronization.

Invitations must have a defined recipient or redemption constraint, expiry, revocation, and one-time acceptance semantics. Concurrent acceptance or repeated qualifying logins must not create duplicate memberships. Account linking follows ADR 0049's verified linking requirement rather than silently merging records by email. Initial administrator provisioning is a separate bootstrap path, not public registration.

Suspending a member must prevent automatic admission from silently recreating or reactivating that membership. Ongoing eligibility, identity-provider changes, member removal, and re-admission require explicit lifecycle rules. The first-login admission decision alone does not establish continuous synchronization with an organization's directory.

Alternatives considered: Admit every member individually, including identity-provider users; or allow optional public built-in registration. The selected model supports organizational onboarding without requiring an open registration surface.

[ADR 0052](0052-locally-managed-human-teams.md) keeps team membership locally managed. Admission does not synchronize identity-provider groups into teams.

Open details: Invitation delivery and redemption; delegated inviter authority; eligibility rule vocabulary; baseline roles; initial administrator setup; changes to eligibility; account removal and re-admission; and lifecycle audit records.

Validation scenarios: Accept a valid invitation once; reject expired or revoked invitations; admit an eligible identity-provider user when enabled; deny automatic admission when disabled or ineligible; prevent duplicate membership and unsafe linking; enforce baseline resource restrictions; and prevent login from undoing a suspension.
