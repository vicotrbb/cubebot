# ADR 0049: Platform-wide built-in accounts and optional OpenID Connect

Status: Accepted

Date: 2026-10-23

Cubebot serves personal installations and organizations with existing identity systems. Human authentication must be consistent across the management console, chat, and installed PWA.

Decision: Support built-in user accounts plus optional OpenID Connect, managed centrally for the entire Cubebot installation. The user selected this model on 2026-10-23. Both sign-in paths use the same platform identity and authorization model across human-facing features. Individual agents, rooms, and frontend surfaces do not maintain separate human account systems.

Usage: An individual operates Cubebot using a built-in account. An organization configures its identity provider for sign-in. A user accesses chat, agent management, memories, trigger configuration, and browser viewing or takeover according to their Cubebot permissions, whether using the web console or the PWA.

Ownership: An identity module within the orchestrator owns human accounts, configured login methods, external identity associations, and platform sessions. An OpenID Connect adapter handles the external authentication flow. Authorization owns roles and resource access. Frontend clients use this shared boundary; they do not independently decide access from login claims. This follows the single-process orchestrator design in ADR 0010 and does not require a separate identity service.

Installation administrators manage authentication configuration. Successful authentication identifies a person but does not automatically confer membership, administrator status, agent management, or private-resource access. [ADR 0050](0050-invitations-and-optional-oidc-admission.md) selects invitations and optional administrator-configured automatic admission through OpenID Connect. Roles and scoped grants follow ADR 0051; [ADR 0052](0052-locally-managed-human-teams.md) keeps human team membership entirely in Cubebot. The existing administrator content-access policy in ADR 0003 applies after the role is assigned.

Platform user records have durable internal identities independent of a particular login method. Connecting another login identity to an existing account requires a defined verified linking process; matching a display name or email alone must not silently merge accounts. Removing or disabling a login method must not transfer ownership of installation-owned agents or confer new authority.

Platform human sign-in is separate from model-provider authentication, external integration accounts, website logins used by agents, and agent-to-orchestrator authentication. A person's Cubebot session does not automatically become an agent credential or authorize use of that person's external accounts.

All protected human-facing operations must enforce the same identity and access decisions at the backend, including streaming and interactive browser controls. The console and PWA share account lifecycle rules; they need not share a literal browser session. Logout, account suspension, session expiry, and role changes require consistent enforcement, with exact session and connection termination behavior specified before implementation.

Alternatives considered: Built-in accounts only, which leaves organizations managing separate credentials; or an external identity provider only, which adds a prerequisite for personal installations. Supporting both meets the intended audiences while preserving one platform account and permission model.

Open details: Admission mechanics and membership lifecycle; built-in login mechanism; initial administrator setup; account recovery and MFA; session storage and expiry; identity linking; supported provider configuration; whether local login can be disabled; emergency access; and human API credential support.

Validation scenarios: Sign in using either configured method; apply identical resource restrictions across console and PWA; deny access based solely on an untrusted role claim; prevent unsafe account linking; enforce suspension across protected operations; preserve agents when a user leaves; and report identity-provider unavailability without bypassing authentication.
