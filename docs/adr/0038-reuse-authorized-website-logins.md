# ADR 0038: Reuse authorized website logins across browser sessions

Status: Accepted

Date: 2026-10-23

ADR 0037 gives the main lane and browser-using tasks separate sessions. Requiring a fresh login for every task would interrupt work even when the agent already has authorized access to the required website account.

Decision: Task sessions may reuse website logins already authorized for their agent and applicable to the task. The user selected this model on 2026-10-23. Reuse preserves separate pages and interaction state; it does not merge sessions or require every task to share the main session's entire browser profile.

Usage: An agent has authorized access to a project-management account. A task can use that account in its own browser session while the main lane works in another session. Both remain subject to the granted account permissions. A task does not receive unrelated accounts merely because they have been used in the main browser.

Ownership: Authorization determines account eligibility for the agent and task. The browser module associates sessions with the intended website account and applies supported reusable authentication state. It must identify which account a session is using and expose expired, mismatched, or unavailable authentication honestly. Credential storage, initial login, and reauthentication interfaces remain to be designed.

Reusable website authentication is sensitive material. It must not be copied into model prompts, ordinary tool-result text, or unrelated session profiles. Reuse does not authorize exporting credentials, sharing them with another agent, or expanding task grants. It is distinct from model-provider authentication under ADRs 0028, 0031, and 0032; those decisions do not automatically make the orchestrator responsible for every website's login or refresh behavior.

Implementation requirements: Reuse only the state needed for the authorized website/account where supported. Do not have concurrent sessions mutate one live profile directory or blindly merge every session's cookies and storage. The choice between initial snapshots, explicitly published authentication updates, or other supported mechanisms remains open. Page state and visual targeting retain the independent-session contract.

Not every website login can be reused in another session. Expired state, account challenges, or unsupported authentication must lead to an accurate need for login or intervention rather than a claim that access was established. Reuse is a supported path when technically possible, not a guarantee of permanent login or concurrent-session acceptance by every website.

Account changes and external actions can affect other sessions using the same account even when their browser state is separate. Logout, revocation, and refreshed authentication require coordinated behavior. New session reuse must respect current grants, while existing sessions follow defined revocation and bounded-disconnection rules. Immediate provider-side invalidation cannot be inferred from deleting a local copy.

Separate task sessions remain an execution-coordination mechanism within one agent and tool environment, not a security barrier against arbitrary code in that environment. The isolation design must state what credential protections it actually enforces. Reuse does not add per-task filesystem isolation contrary to ADR 0005.

Alternative considered: Independent login for every session. That would separate initial authentication state but introduce repeated login work and potential human intervention for ordinary tasks. The user selected reuse of authorized logins.

Consequences: Agents can perform authenticated browser tasks without unnecessary repeated login. Cubebot must track account attribution and handle shared-account effects. [ADR 0039](0039-browser-live-view-and-takeover.md) permits authorized users to take over a session, including for login assistance. Persistence across sleep, secret storage, and session cleanup remain open.

Account administration: [ADR 0043](0043-installation-managed-external-account-connections.md) makes reusable external account connections installation-managed. Browser-assisted login does not itself grant authority to register or share a new connection.

Open details: Account grant schema; secret storage; supported authentication-state formats; update ownership and versioning; reauthentication; persistence and deletion; account-level concurrency; revocation propagation; and credential isolation from arbitrary tool programs.

Validation scenarios: A task reuses an authorized account while keeping separate pages; an unrelated account is not inherited; a task with insufficient grants is denied reuse; expired or non-reusable state requests intervention; account identity is visible without exposing secrets; concurrent sessions do not write one live profile; and revocation prevents new unauthorized reuse.
