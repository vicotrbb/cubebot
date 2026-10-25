# ADR 0081: Installation-wide connection access by default

Status: Accepted

Date: 2026-10-25

Decision: Keep all integrations and connections installation-managed and make enabled, approved connections usable by all eligible installation agents by default. Each connection has an access setting: all agents, or specific grants. All agents is the default for newly configured connections. The user explicitly selected this change on 2026-10-25.

This amends the agent-eligibility assumptions in ADRs 0028 and 0040–0045. Installation ownership, administrator-managed configuration, tool/resource restrictions, and on-demand discovery remain in force. Earlier statements requiring a separate per-agent assignment apply to connections configured for specific grants.

Usage: An administrator configures a repository integration and an account connection. With all-agents access, existing and newly created agents can discover and use its approved tools under the connection's configured scope without individual assignment. The administrator can instead select specific grants for a sensitive account and authorize only selected agents within that scope.

The policy applies consistently to model-provider connections, external integration accounts, reusable website accounts, and approved credential-free integration connections. The integration definition identifies the implementation and approved tool set; each configured connection identifies its endpoint/account and effective access policy. A connection without credentials still has an access policy.

All-agents access is a dynamic baseline, not a copied grant on each agent. It includes agents created later, including task-bound workers, subject to their applicable execution constraints. These workers obtain eligibility from installation policy, not by inheriting their parent's credentials or memory. Suspended or retired agents do not regain execution eligibility through this default.

For integrations, baseline access includes use of the administrator-approved tools and resource scope configured on the connection. Do not require an additional per-agent tool assignment merely to make all-agents access functional. Specific grants may select a subset of those tools and resources. Installation and execution restrictions and upstream credential scopes still constrain both modes. Newly advertised server tools remain unapproved until administrator approval; once approved, the current connection audience can use them.

Access is distinct from connection health and enablement. A disabled, revoked, or invalid connection remains unavailable in either mode. Administrators configure access modes; delegated managers may issue specific grants only within existing assignment authority. This change does not grant ordinary members or agents permission to register integrations, alter endpoints, broaden policy, or inspect raw credentials.

Use the same effective-access evaluation for discovery, provider/model selection, website account selection, credential issuance, and invocation. When all-agents mode is turned off, only independently valid specific grants remain effective. When turned on, current and future eligible agents receive the baseline. Do not retain stale copied defaults as grants.

Policy changes are versioned and audited. Recheck authorization before starting operations and reconcile catalog and credential caches. Stop issuing or refreshing credentials to agents that lose eligibility. Already-issued effects and bounded disconnected authority retain ADR 0012's limitations; a policy change must not be displayed as instantly undoing external actions. Exact connection revocation mechanics remain qualification work.

Connection availability does not automatically choose a model, switch accounts, start a tool process, distribute every secret to every pod, or load every tool schema into a prompt. Keep explicit account/model selection, purpose-scoped credential delivery, and on-demand tool discovery. Existing agent data and human content permissions are unaffected.

Alternative considered: Require an explicit assignment for every agent and connection. The user replaced that default with installation-wide availability and optional restricted access.

Proposed schema: access_mode = all_agents | explicit_grants, plus the existing enabled state, approved tool/resource scope, and policy revision. Per-agent exclusions in all-agents mode are not introduced by this decision; restricted access uses explicit-grants mode. Existing deployed restricted records, if any, must not be silently broadened by a future migration. This repository currently records a design change, not a live configuration mutation.

Validation scenarios: New connections default to all agents; existing and new agents can discover and use approved capabilities without assignment; restricted connections require grants; turning the default off removes baseline eligibility; stale caches cannot authorize new operations; approved tool additions follow the current audience; new unapproved tools remain unavailable; disabled connections stay unavailable; and broad usability does not expose raw secrets or change model/account selection.
