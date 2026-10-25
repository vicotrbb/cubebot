# ADR 0003: Administrator content access

Status: Accepted

Date: 2026-10-22

Under [ADR 0001](0001-installation-and-tenancy.md), one individual or organization owns each installation. [ADR 0002](0002-agent-ownership.md) establishes installation-owned agents with assigned managers and scoped access. Administrator access to private content needs an explicit rule within that model.

Decision: Installation administrators have full content access by default. The user selected this model on 2026-10-22. Administrators can inspect all agent conversations, memories, and workspace files, including content private to ordinary members. No exceptional-access override or additional content grant is required for an administrator.

Rationale: The selected model gives installation administrators direct access for organizational oversight and troubleshooting, using one ordinary administrative role rather than an exceptional-access workflow.

Alternatives considered: Scoped access with an explicit audited override would make private-content inspection a separate deliberate action. Scoped access with no administrator override would require an ordinary content grant. Both alternatives were declined in favor of default administrative access.

Consequences: Direct access to private content is restricted for unauthorized ordinary members, not installation administrators. Product language and permission displays must communicate that distinction. An agent manager is not automatically an installation administrator. This decision does not grant ordinary users or agents direct access to other users' private content. Under [ADR 0004](0004-unified-agent-memory.md), an agent may nevertheless reuse knowledge learned from private conversations in other contexts; transcript access restrictions do not isolate its knowledge.

Scope: This decision concerns reading agent content. Credential disclosure, credential use, destructive operations, role assignment, and audit logging remain separate decisions. Full content access does not by itself settle those permissions.

Validation scenarios: An administrator can inspect private agent content without a special override; an ordinary member without a content grant is denied; assigning an agent manager does not implicitly grant installation-wide content access. Role revocation behavior will be specified with the authorization model.
