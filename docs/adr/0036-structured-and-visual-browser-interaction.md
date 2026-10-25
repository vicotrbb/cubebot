# ADR 0036: Structured browser tools with visual interaction

Status: Accepted

Date: 2026-10-23

Agents need to read websites, complete forms, and test interfaces. Browser interaction must handle both accessible page structure and work that requires visual inspection without creating separate, inconsistent browser states.

Decision: Provide structured browser tools as the default interaction path, with screenshots and visual actions available when needed. Both operate on the same browser session. The user selected this model on 2026-10-23. This choice does not select a browser library or provider-specific computer-use engine.

Usage: An agent inspects a page, locates a field by its label, and fills it through a structured action. When the task requires examining a visual layout or interacting with an element that lacks useful structure, it can inspect a screenshot and perform a targeted visual action in that same session. It then observes the resulting state before claiming success.

The structured interface should support bounded page inspection and actions on observed elements. The visual interface should support screenshots and relevant pointer, keyboard, and scrolling actions. Both use explicit session and page identity. Exact tool names, schemas, script-evaluation capabilities, and observation formats remain open.

Ownership: A browser module in the tool-execution environment owns sessions, observations, action targeting, and browser outcomes. The runtime invokes its interface under the same authorization and execution accounting as other tools. Browser processes remain in the tool container under ADR 0033. The runtime's provider credentials do not become browser credentials.

Structured and visual paths share permission and network enforcement under ADRs 0013 and 0035. A visual click does not bypass a restriction applied to structured actions. Page content is untrusted task data and cannot grant access, alter runtime instructions, or authorize new operations.

Observations and actions must refer to the intended page state. Navigation, changed elements, or a stale screenshot can invalidate a target. The implementation must detect relevant invalidation and re-observe rather than blindly reuse stale coordinates or element references. Browser operations with uncertain outcomes require inspection before retry, especially submissions with external effects. A successful click alone does not establish that the intended business operation completed.

Execution records associate actions and results with the responsible agent, lane, and session. Screenshot and page-content capture must respect output budgets and retention rules. These observations enter the requesting lane as tool results; this decision does not mandate recording continuous browser video or every page byte.

Visual work requires a configured model capable of consuming the required observations. Model capability validation must expose an incompatible configuration honestly. ADR 0030 still prohibits silently switching models. Structured work can remain available when supported even if a specific visual operation cannot proceed.

Alternatives considered: Visual-only interaction would use one action model but make ordinary text and form work depend on screenshot interpretation. Structured-only interaction would reduce the tool surface but provide no visual path when page structure is insufficient. The user selected both with structured interaction as the default.

Consequences: Cubebot must maintain consistent targeting, grants, and records across both interaction paths. [ADR 0037](0037-main-and-task-browser-sessions.md) assigns separate browser sessions to the main lane and tasks that need browsing. [ADR 0038](0038-reuse-authorized-website-logins.md) permits reuse of applicable authorized website logins across those sessions. Profile persistence and credential storage remain open. Browser visual interaction does not by itself promise control of a complete operating-system desktop.

Human interaction: [ADR 0039](0039-browser-live-view-and-takeover.md) provides live viewing and explicit human takeover. Both structured and visual agent actions obey the selected session's exclusive control state.

Open details: Browser implementation and Rust integration; tool schemas; observations and stale-target handling; model capability requirements; session allocation mechanics and persistence; browser authentication; live transport and handoff mechanics; downloads and uploads; and capture retention.

Validation scenarios: Read a page and fill a labeled form; use a screenshot and visual action in the same session; reject a stale target after navigation; inspect an uncertain submission before retrying; enforce private-network restrictions across both paths; reject unsupported visual inference without model fallback; and attribute browser actions to the correct execution lane.
