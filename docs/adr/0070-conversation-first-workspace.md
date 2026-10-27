# ADR 0070: Conversation-first workspace

Status: Accepted

Amended by [ADR 0082](0082-one-permanent-conversation-per-agent.md): each agent has exactly one permanent conversation and manages context within it. Earlier references to multiple conversations for the same agent are superseded.

Date: 2026-10-23

Decision: Organize the web console and installed PWA around conversations with agents and rooms. The user selected this direction on 2026-10-23. Goals, assignments, artifacts, memory, and skills are accessible in the context of a conversation. Work overviews and fleet administration remain available as supporting destinations.

Usage: A person selects an agent or room, reads the conversation, sends a message, follows agent activity, and opens a related goal or artifact without losing their place. They can inspect an agent's memory, installed skills, and visible dreaming results through contextual navigation, subject to the existing access rules.

Desktop direction: Use navigation for agents and rooms, a central conversation, and contextual detail panels. Panels should open when useful rather than consume space permanently. Exact navigation groups, widths, and visual styling remain design work.

Mobile direction: Preserve the same destinations and capabilities using a focused conversation view and navigable detail screens or sheets. Preserve drafts and conversation position when moving between them. Do not compress a desktop multi-column layout into a narrow viewport.

Ownership: Frontend navigation and presentation compose orchestrator-authorized data under ADR 0069. A room membership or visible message does not grant access to every linked artifact, agent memory, skill operation, or management control. Each destination uses the applicable resource authorization. Unified memory under ADR 0004 remains agent-owned across conversations; the interface must not imply that opening another conversation creates a separate memory pool.

Conversation history presents messages and relevant work updates. Goals and assignments retain their durable identities and inspectable state independently of their appearance in chat. Link progress, questions, outcomes, and artifacts to the work that produced them. Detailed operational records can be inspected separately so concurrent activity does not overwhelm the conversation.

The conversation view must distinguish accepted messages, pending sends, queued work, active work, and failures. Agent state and task state may differ because an agent can run multiple lanes under ADR 0005. Switching conversations or closing a panel does not stop work. [ADR 0071](0071-queued-messages-and-explicit-steering.md) selects queued messages with explicit steering of active work. [ADR 0072](0072-cancellation-of-delegated-work.md) scopes cancellation to selected work and its descendants.

Alternatives considered: A work-first inbox makes goals and outcomes the entry point; a fleet-first dashboard prioritizes status and operations. The user selected conversations as the primary entry point. Supporting work and fleet views can still serve their respective workflows.

Related decisions: ADRs 0014–0016 define room routing and collaboration; ADR 0025 requires visible dreaming results; ADRs 0053–0054 define connectivity, drafts, and notifications; ADR 0069 establishes the frontend stack, performance, and accessibility requirements.

Open details: Navigation grouping and search; task conversation presentation; contextual panel behavior; deep links and mobile back navigation; visual design; and presentation of work controls. This decision does not select threading, a component library, or a fixed pixel layout.

Validation scenarios: Move between a direct agent conversation and a room; inspect a linked goal or artifact and return to the same scroll position; preserve a draft while opening memory details; navigate equivalent mobile flows; distinguish an agent's concurrent tasks; handle inaccessible links honestly; and keep conversation input responsive under the workload defined in ADRs 0061–0062. Implementation and UI qualification are pending.
