# ADR 0002: Agent ownership

Status: Accepted

Date: 2026-10-22

Following [ADR 0001](0001-installation-and-tenancy.md), one individual or organization owns each installation. Agent ownership must support private assistants and shared specialists, including continuity when their creator leaves.

Decision: All agents belong to the installation, with assigned managers and scoped access. The user selected this model on 2026-10-22. Creating an agent does not make its lifecycle depend on the creator's account. A private assistant and a shared specialist use the same ownership model.

Managers administer agents according to permissions. Ownership alone does not grant every installation member access to an agent. Management authority, conversation access, memory access, workspace access, and credential use require separately defined permissions. Installation ownership does not transfer ownership of a user's provider credentials or preserve authorization after those credentials are revoked.

Alternatives considered: User-owned agents would require ownership transfer or another lifecycle decision when a user leaves. Supporting both user-owned and installation-owned agents would introduce separate ownership categories and transfer rules. The selected model supports the intended personal and organizational deployments with one lifecycle model.

Consequences: An agent's durable identity and history can remain after its creator's account is removed. Continued execution still depends on valid managers, permissions, credentials, and operating policy. Account removal must not silently confer new authority on the agent or other users.

Related decisions: [ADR 0003](0003-administrator-content-access.md) grants installation administrators full content access by default. Agent management alone does not confer this role. [ADR 0028](0028-installation-managed-provider-connections.md) selects installation-managed provider connections only; external account ownership and authorization remain distinct from ownership of Cubebot records.

[ADR 0051](0051-built-in-roles-and-scoped-resource-grants.md) selects built-in roles and scoped resource grants for users and teams, without installation-defined custom roles initially.

Open details: The exact permission matrix, manager assignment and transfer rules, behavior when no manager remains, and handling of active work during revocation will be decided separately.

Validation scenarios: A creator leaves while an agent has an active task; a new manager takes responsibility without automatically acquiring unrelated private content; a credential behind an installation-managed provider connection is revoked; an agent is shared with a team under explicit grants. Expected transitions will be specified with the permission and execution models.
