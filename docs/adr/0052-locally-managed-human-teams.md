# ADR 0052: Human teams managed entirely in Cubebot

Status: Accepted

Date: 2026-10-23

Human teams group installation members for scoped access under ADR 0051. Cubebot needs a single authority for that membership when users may sign in through either built-in accounts or OpenID Connect.

Decision: Manage human teams and their membership entirely in Cubebot. The user selected this model on 2026-10-23. Authorized humans add and remove members locally. Identity-provider groups do not create, synchronize, or change Cubebot team membership. OpenID Connect remains responsible for sign-in and optional admission under ADRs 0049 and 0050.

Usage: An administrator creates an Engineering team, adds admitted installation members, and grants the team access to selected agents and rooms. A member's external group changes do not alter that local membership. A user admitted automatically through OpenID Connect receives the configured baseline authority and joins teams through Cubebot's local management process.

Ownership: The orchestrator's identity and membership module owns team records and membership changes. Authorization owns team-scoped grants and evaluates them against current local membership. Team administration uses the platform's shared human identity and authorization boundary. Exact delegated team-management powers remain to be specified.

Human teams are access-control groups, distinct from rooms, agent teams, or delegated agent work. Creating a human team does not create agents or a conversation. The installation continues to own agents regardless of who belongs to a team with access to them.

Team changes must record the actor and affected membership and support explanation of effective access. Removing a member removes access derived from that membership according to the revocation contract. The person may retain access through a separate direct grant or another team; the console must make those remaining sources understandable. Suspension of an installation member prevents use of team-derived access even if membership records remain.

Managing team membership and assigning grants to a team are separately authorized operations. Adding someone to a privileged team conveys its existing access, so delegated membership administration must respect the permitted scope. The exact role matrix and delegation limits remain part of ADR 0051's specification.

Alternatives considered: Local teams with optional identity-provider group mapping, or exclusively provider-managed teams. Local management gives both login methods the same team workflow and avoids synchronization and conflicting membership sources in the initial design.

Open details: Team creation and deletion rights; delegated membership administration; team deletion effects on grants; membership audit presentation; and revocation timing for active sessions and operations.

Validation scenarios: Add and remove members locally; apply team-scoped grants; preserve local membership across external group changes; prevent login from restoring a removed team membership; deny unauthorized membership changes; explain access through multiple grants; and deny suspended users despite retained membership records.
