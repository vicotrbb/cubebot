# ADR 0051: Built-in roles with scoped resource grants

Status: Accepted

Date: 2026-10-23

Cubebot needs consistent human permissions for administration, agent management, conversations, memory, workspaces, and interactive controls. Organizations also need to share selected resources with teams without granting installation-wide authority.

Decision: Use Cubebot-defined roles and permissions with scoped resource grants assigned to users or teams. The user selected this model on 2026-10-23. Installation-defined custom roles are outside the initial design. Exact built-in role names and their permission matrix remain to be specified.

Usage: An administrator grants a team conversation access to a particular agent and its shared room. A designated manager can administer that agent within their assigned scope. Another user may inspect memory without editing it. These distinctions must be expressible through the built-in roles and supported scoped grants, without requiring a custom role definition.

Ownership: The authorization module owns role definitions, grants, and effective-access evaluation. Identity and team membership supply the human subjects to which grants apply. Domain modules enforce authorization for their operations through this shared boundary. The console exposes assignments and effective access; frontend visibility alone does not enforce permissions.

A grant identifies its recipient, permitted built-in role or supported permission, and resource scope. Installation-wide administration remains distinct from managing a particular agent. An agent manager does not become an installation administrator, and conversation access does not implicitly include memory editing, workspace modification, browser takeover, or credential administration. The detailed matrix must preserve the separations already selected in earlier ADRs.

ADR 0003's full administrator content access remains in force. Administrator access to content does not by itself settle credential disclosure or every destructive operation. Initial membership and its baseline role under ADR 0050 do not automatically confer access to private resources.

Grant-management authority must itself be scoped. A delegated manager cannot confer more authority than they are allowed to assign. Access evaluation must account for current user status, team membership, and resource grants, and produce an explainable result. Combination rules, revocation propagation, and any explicit-deny semantics remain to be designed; this decision does not introduce a general policy language.

Human roles are separate from agent execution grants. Giving a person access to chat with an agent does not create new tool permissions for that agent. Likewise, agent tool access does not grant a human access to its stored credentials or unrelated content. Unified agent memory under ADR 0004 remains a shared knowledge pool across that agent's work; transcript restrictions do not create separate memory partitions.

Alternatives considered: Built-in roles plus installation-defined custom permission bundles. Custom roles offer organizational naming and reuse but add configuration, migration, and explanation requirements. Built-in roles with scoped grants are selected for the initial product.

[ADR 0052](0052-locally-managed-human-teams.md) makes human team membership entirely local to Cubebot, without identity-provider group synchronization.

Open details: Role and permission matrix; resource scope types; delegated team administration; grant combination and delegation rules; access explanations; and revocation behavior for active sessions and operations.

Validation scenarios: Grant team access to selected resources; deny access outside the grant; separate reading from editing and viewing from takeover; prevent a manager from assigning greater authority; preserve administrator content access; explain access inherited through a team; and enforce membership or grant removal according to the defined revocation contract.
