# Console design

Confirmed 2026-11-01. This is the intended web console and PWA under [Dedicated TanStack Start frontend with Bun and Vite](../adr/console.md#dedicated-tanstack-start-frontend-with-bun-and-vite), [Conversation-first workspace](../adr/console.md#conversation-first-workspace) and [One permanent conversation per agent](../adr/agents.md#one-permanent-conversation-per-agent); [STATUS](../STATUS.md) says which parts run. The console is an Operate surface: a person is in a task, so Slack's grammar (sidebar, one conversation in the middle, contextual panels, threads) is followed straight, at Slack's craft level, with Linear as the bar for management screens. The visual world is the landing site's desk world (see `DESIGN.md`) in a product register: warm paper ground, orange as the one accent, sage for sleeping, Rubik, block buttons with a bottom edge, warm blurred shadows, and the cube as the only illustration. No scenery. A dark theme is the same desk at night.

## Who and what for

- The administrator who runs the installation and lives in the console; and a member who holds scoped grants and opens it to work with a few agents. Both arrive mid-task: ask an agent for something, read what it did, or step in when it asks for a person.
- The primary action is sending a message to an agent or room and watching the work land. Evidence is already in the ledger (origin, tool calls, delegated children, steering, journals, dreams, routine runs, desktop sessions, handoffs); the console puts it where Slack puts attachments and threads.
- Everything is permission-shaped. A person sees an agent when they hold any capability on it, and only the tabs those capabilities allow. Administrators see everything.

## Desktop topology

- **Sidebar** (260 px, collapsible to a 56 px rail): installation name, search (⌘K palette), the person's menu. Sections, collapsible and remembered per person: Activity, Favorites, Agents, Rooms, Files, Skills, and Platform for administrators (People, Teams and access, Sign-in, AI providers, Vault, Audit log, Health). An agent row is its cube avatar, name and activity; unread rows are bold with an orange mark.
- **Conversation**: header with the animated cube, name, activity word, and on the right Desktop (live dot while the agent uses it), Details, and a kebab. Messages are grouped by author like Slack. Tool calls are compact work cards under the message, collapsed by default, expandable to arguments, output and journal state; uncertain results say so. Delegated work is a thread: "3 tasks, 2 done" opens the children in the right panel. Steering is a reply in that thread while the turn runs. Origin lines read "via room Support" or "via routine Nightly digest". Routine command output and screenshots are work cards.
- **Composer**: attach from shared files, a queued-messages strip while a turn runs, Cancel on the running turn. Enter sends, Shift+Enter breaks. Drafts persist locally per installation, account and conversation ([Online console operations with local drafts](../adr/console.md#online-console-operations-with-local-drafts)).
- **Details panel** (right, 380 px, every tab a URL): About and manage, Memories, Dreams, Skills, Routines, Vault, Files, Access, Status. Rooms get Members and Routing.
- **Agent management** (About and manage): identity (name, avatar builder, wallpaper); instructions with revision history; behaviour (idle delay before sleeping, task lanes, dreaming model); lifecycle (start, stop, restart, retire, with the observed pod state, blocked reason and drain report); access (grants on this agent); tools (installation flags, with MCP servers managed on the Connections tab); danger zone. **Connections** (tab, `agent.manage`): MCP servers with a per-agent switch for servers limited to chosen agents (a server shared with every agent is changed under Platform › MCP servers), then connected accounts, empty until integrations exist.
- **Floating live preview**: when any agent whose desktop the person may view starts computer or browser work, a 320×200 movable window appears top right with its cube, name, live tiles and latency. Click expands to the Desktop route; dismiss hides it until that agent's next desktop session; several stack. The Desktop route keeps take control, hand back, vault fill, the login banner and who holds control.
- **Rooms** are channels: members strip, coordinator decisions as system lines ("Coordinator assigned this to @Keeper"), deliveries with status.
- **Platform** screens are dense tables with a right-side editor: accounts and invitations (token shown once), teams and membership, grants with "explain access", OIDC bindings, providers, installation vault items, audit events with cursor paging, and health (controller mode, ownership, agents by pod phase).

## PWA topology

Bottom bar: Home, Activity, Files, You. Home is the sidebar as a list. A conversation is full screen with a back control; tapping the header opens Details as a sheet. The floating preview becomes a pill under the header ("Keeper is using its desktop · watch") that opens the view-only desktop. Installable manifest and an app-shell service worker only; no offline history ([Online console operations with local drafts](../adr/console.md#online-console-operations-with-local-drafts)).

## Avatars

One `Cube` component ported from the site. Stored on the agent: `paint` (orange, sage, paper, wood, sky, rose, lemon, plum), `eyes` (round, wide, sleepy, dot), `accessory` (none, cap, glasses, bow, antenna). Mood is derived from activity, never stored: idle blinks and glances; working looks down with a typing rhythm; dreaming has half-lids and slow drift; sleeping (idle past its delay, still running) is closed eyes with z's; stopped is closed eyes on dimmed paint with no z's; desktop-active adds a faint screen glow on the front face; retired is a raw wood block without eyes. Reduced motion freezes every pose.

## States and ranges

0 to 100 agents and 0 to 30 rooms; favorites and search make 100 usable. Conversations render windowed from the newest with "earlier" paging. Empty states teach (no agents yet; no access yet; empty room; no memories; no routines). Loading is skeletons in place. Error and waiting turns keep Retry; uncertain tool results stay visible and block retry. Offline shows a banner and keeps the draft; nothing is sent on reconnect by itself.

## API contract

The contract the console needs beyond what the orchestrator serves today. Runtime (agent-facing) routes stay out of the browser document.

1. **OpenAPI coverage** of routes the router already serves: `GET /api/agents/{id}`, `POST .../lifecycle`, `GET .../status`, `GET .../revisions`, `GET/PUT .../wallpaper`, every `/api/identity/*` management route (accounts, suspend, invitations, teams, membership, grants, revoke, explain, events, command receipts, OIDC bindings, revoke, start, link), `GET /api/health/live` and `ready`.
2. **Agent summary** (`GET /api/agents`, `GET /api/agents/{id}`): returns every agent the caller holds any capability on, or all for administrators; adds `capabilities` (the caller's own, as grant names such as `conversation_send`; administrators get the full list), `avatar`, `desktop_active` (a running turn has called a desktop tool, `computer`, `browser_*`, `vault_fill`, `request_login` or `screenshot`, in the last 120 s), `pending_login`, `last_sequence` (highest turn sequence in its conversation) and `read_sequence` (the caller's cursor, 0 if none). `instructions` is `null` without `agent_manage`. Create and update accept optional `avatar`; it is not part of revision history, like the wallpaper.
3. **Account**: `GET /api/identity/me` adds `login` and `name`. `GET/PUT /api/identity/me/preferences` holds `{favorites: [{kind: "agent"|"room", id}], theme: "system"|"light"|"dark", collapsed: string[]}`; last write wins; stored as one JSON document per account.
4. **Read cursors**: `PUT /api/conversations/{id}/read` and `PUT /api/rooms/{id}/read` with `{sequence}`, monotonic per account and scope. Room summaries add `last_sequence` and `read_sequence`.
5. **Installation stream**: `GET /api/events` (SSE, polled once a second like the conversation stream) with `agent` frames carrying the agent summary whenever its `activity`, `desktop_active`, `pending_login` or `last_sequence` changed, `room` frames `{id, last_sequence}`, and keep-alives. One per browser session; it replaces per-conversation polling for the sidebar and the floating preview.
6. **Activity inbox**: `GET /api/activity` lists, newest first and at most 100, items `{kind, agent, room, turn, at, text}` with kinds `login_pending`, `turn_waiting`, `turn_failed`, `invitation_pending` (administrators) and `dream_published_skill`. Derived from existing tables; no new storage.
7. **Installation**: `GET /api/installation` returns `{name, terminal_enabled, computer_use_enabled, controller: "kubernetes"|"docker"|"database_only"}`; `name` comes from `CUBEBOT_INSTALLATION_NAME`, default "Cubebot".
8. **Later**: `GET /api/agents/{id}/usage`, `GET /api/agents/{a}/memories/{m}/revisions`, `GET /api/search?q=`, notifications and push ([In-app notifications with optional push](../adr/console.md#in-app-notifications-with-optional-push)).

## Build order

1. Backend: items 1 to 7, then `just api` so `apps/web/src/api/schema.d.ts` carries the contract.
2. Web foundation: tokens and themes, `Cube`, app shell with file routes, sidebar, sign-in, command palette.
3. Conversation, composer, work cards, threads; details panel with the ported Memories, Dreams, Skills, Routines, Vault and Files tabs.
4. Desktop route and floating preview; rooms; files and skills library; platform screens; agent management.
5. PWA manifest, service worker, bottom bar and sheets.
6. Update the `tests/system/*_browser.mjs` harnesses to the new structure; refresh STATUS and the reference.

## Routes

`/sign-in`, `/` (redirects to the first favorite or agent), `/activity`, `/agents/$agentId` with child tabs `/about`, `/memories`, `/dreams`, `/skills`, `/routines`, `/vault`, `/files`, `/access`, `/status`, and `/desktop`; `/rooms/$roomId` with `/members`; `/files` and `/files/$folderId`; `/skills` and `/skills/$skillId`; `/platform/people`, `/teams`, `/access`, `/sign-in`, `/providers`, `/vault`, `/audit`, `/health`; `/you`.

## Tasks (2026-11-04)

`/tasks` asks one question, "What do you want done?", above the person's tasks grouped as Needs you, Working and Finished. `/tasks/$taskId` is the task's room with its plan beside it, above it on a phone. The sidebar's Tasks row gains a chevron while tasks run, opening to a tree of them; the choice is kept in preferences. Platform › Tasks chooses the orchestrator's model, its limits and its house rules. The orchestrator wears the brand's orange block with an antenna, the only cube not chosen by a person. See [Tasks and the task orchestrator](../adr/rooms.md#tasks-and-the-task-orchestrator).

Live desktops stack into one floating window with the others peeking behind it, and its counter opens a list to choose which to watch; only the front one holds a connection. A phone shows one line with the same list.

Work reads as a rail of sentences ("Wrote subtitles.ts in finder"), each opening to highlighted code, a diff or a terminal, which stays dark in either theme.

## Situation room (2026-11-04)

`/situation`, below Broadcast in the sidebar and on a phone's Home page: everything happening on the installation for the agents the viewer may read. It is the one screen that is an overview rather than a conversation, so it is a ruled ledger, not a wall of tiles: one sentence on what needs a person, or that nothing does; Spend (today, 7 days, this month, 30 daily bars, models); Agents, each with what it is doing and its month against its budget; and a rail of Now and Latest. The page itself never scrolls: it fills the pane and each list (Agents, Now, Latest, Models) scrolls in its own space, with Platform a one-line strip at the bottom. A pane too short to hold that scrolls as a whole rather than crushing the lists. Too narrow for the sections side by side, it shows the sentence and one view at a time: Now, Agents or Spend. Colour stays for what wants attention, and state is always in words too. An agent's Usage tab reuses the same meter and bars and holds the budget.
