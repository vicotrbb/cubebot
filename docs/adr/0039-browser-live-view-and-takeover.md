# ADR 0039: Live browser viewing and explicit human takeover

Status: Accepted

Date: 2026-10-23

Agents have separate main and task browser sessions and can reuse authorized website logins. Users need to inspect ongoing browser work and directly assist with login or an interaction the agent cannot complete.

Decision: Provide live viewing and explicit, exclusive human takeover of a selected browser session through the web console and mobile PWA. The user selected this model on 2026-10-23. Viewing and controlling have separate permissions. The console shows the selected session and its current controller.

Usage: A user watches an agent's task session, requests control, completes a website login, and returns control. Once the handoff is confirmed, the agent observes the current page again and continues from the user's resulting state. Unrelated browser sessions may continue under their existing grants and resource limits.

Ownership: Authorization owns viewing and control permissions. The browser module owns exclusive input authority and enforcement for each session. The orchestrator coordinates authenticated console requests and records control transitions. The console presents live state and submits user input only while authorized control is held. Stream transport and routing remain open.

Takeover is a confirmed transition, not merely a UI toggle. Stop admitting agent browser actions for the selected session and settle or reconcile in-flight actions before granting human input authority. Pending structured actions and visual input must obey the same ownership rule. A submitted external action cannot be undone by taking control, so unresolved outcomes must remain visible.

At most one controller may issue input to a session at a time. Multiple viewers do not gain control implicitly. A stale client or queued agent action must not act after its authority has been replaced. The concrete ownership token, lease, and reconnect protocol remain implementation choices. If a disconnected runtime cannot be fenced from controlling the session, the console must not falsely claim takeover succeeded.

Returning control invalidates stale observations and planned browser actions. The agent re-observes the page and accounts for visible user changes before further interaction. The handoff must not restart the original task or replay already-completed submissions. A lost viewer connection is not an explicit return of control; timeout and abandoned-session recovery policy remain to be specified.

The selected session pauses agent interaction during takeover. Other sessions may continue, but they still share external account effects and workspace resources under ADR 0037. The agent must not bypass the pause through another automation path into the controlled session. Enforcement against arbitrary programs in the shared tool environment is part of the browser/tool design, not guaranteed merely by hiding browser tools from the model.

Live viewing exposes website content and must follow content permissions. Controller access can exercise the website account's authority, so it is separate from permission to read recorded agent content. Login secrets entered during assistance must not be copied into model messages or ordinary action logs. Capture and redaction rules remain to be designed. A login during takeover does not automatically authorize sharing that account with other agents or tasks beyond ADR 0038's grants.

Record controller identity, session identity, takeover and release times, and relevant outcomes. This decision does not require permanent video recording or raw keystroke retention. The live UI must identify stale or disconnected views and unavailable sessions honestly. Browser interaction on mobile requires appropriate touch and keyboard controls.

Alternatives considered: Live viewing without input would allow observation but not direct assistance. Recorded screenshots and action history would reduce streaming needs but provide less immediate support. The user selected live viewing and takeover.

Consequences: Cubebot needs live media delivery, input arbitration, and handoff recovery. Active human control must be accounted for before an ordinary idle shutdown; exact timeout and keep-awake behavior remain open. This feature does not introduce mandatory per-action approval for autonomous agents.

[ADR 0053](0053-online-console-with-local-drafts.md) requires online console operations. Disconnected browser input is not queued for replay; reconnecting requires establishing current control ownership and page state.

Open details: Live transport and deployment routing; permission mapping; input ownership and fencing; in-flight action handling; disconnect timeouts; sleep interaction; capture redaction and retention; mobile interaction; and enforcement across alternate automation paths.

Validation scenarios: View without control permission; take over one session while unrelated tasks continue; reject simultaneous controllers and stale input; reconcile an in-flight click before handoff; resume after observing user changes; handle a dropped mobile connection without dual control; avoid exposing login secrets in logs; and refuse to report takeover before execution ownership is confirmed.
