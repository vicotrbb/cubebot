# ADR 0001: Installation and tenancy

Status: Accepted

Date: 2026-10-22

Cubebot must serve individuals and organizations. The installation model determines whether unrelated organizations share one control plane and affects identity, authorization, credentials, data ownership, execution isolation, and operational responsibility.

Option A gives each installation one owning individual or organization. An organization can have multiple users, teams, private rooms, agents, and scoped integrations. Internal access controls remain necessary. Independent organizations operate separate installations, which may share underlying infrastructure if its isolation policy permits.

Option B lets one installation host multiple independent organizations. A platform operator manages shared infrastructure, while organization administrators manage their own users and agents. Tenant isolation must apply throughout storage, scheduling, credentials, memory retrieval, tool execution, and operational access. The operator's access to tenant data must be defined separately.

Decision: Option A. The user selected one individual or organization per installation on 2026-10-22. Each installation has one owner. Multiple users, teams, rooms, and agents may operate within it, subject to internal permissions.

Rationale: This directly serves the self-hosted audience and reduces the isolation and administration rules the initial implementation must own. Shared tenancy for unrelated organizations is outside the initial design. A future hosted offering can use separate installations or introduce shared tenancy through a later ADR. Shared tenancy is not guaranteed to be a configuration-only change.

Related decision: [ADR 0002](0002-agent-ownership.md) establishes installation-owned agents with assigned managers and scoped access.

Related decision: [ADR 0003](0003-administrator-content-access.md) grants installation administrators full content access by default.

Related decision: [ADR 0049](0049-platform-wide-human-authentication.md) provides installation-wide built-in accounts and optional OpenID Connect for human sign-in, with a shared identity and authorization model across the platform.

Open details: Other administrator powers, member permissions, and infrastructure isolation will be decided separately. One owning organization does not imply that every member or agent can access all organization data. This decision does not require a dedicated Kubernetes cluster per installation.

Validation scenarios will include an individual managing personal agents, colleagues using private and shared rooms, an agent attempting to retrieve unauthorized memory, and credential revocation during active work. These scenarios will be refined as the permission model is decided.
