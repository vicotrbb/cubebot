---
version: 1
slug: 'apps-web-src-routes-root-tsx'
primary_target: 'apps/web/src/routes/__root.tsx'
related_targets:
  ['apps/web/src/routes/index.tsx', 'apps/web/src/features/chat.tsx']
---

# Surface: Cubebot console (apps/web)

Mode: Operate. Scope: the whole console and PWA.

Audience and job: the administrator who lives in the installation and the member with scoped grants; both arrive to message an agent or room, read its work, or step in. Proof is the ledger's own evidence rendered where Slack renders attachments and threads. Constraints: permission-shaped everywhere, TanStack Start file routes, one component system, Playwright harness hooks kept, complexity gate, no invented capabilities.

## Direction contract

THESIS: Slack's grammar played straight for agents instead of people: sidebar, one conversation, contextual right panel, threads for delegated work. It refuses the dashboard-first console (tiles, metrics, status grids) and the chat-widget console (a lone chat with buttons bolted on).

OWN-WORLD: the desk world in a product register. Warm paper ground (desk) with a slightly deeper warm sidebar, orange as the only saturated color for selection, primary actions, unread marks and focus, sage for sleeping, wood and paper neutrals, warm blurred shadows, Rubik throughout at a fixed rem scale, block buttons with a 3 px bottom edge, 8 to 10 px radii, lucide icons at one stroke. Dark theme: warm near-black ink ground, orange unchanged, sage dimmed. The 3D cube with eyes is the only illustration and carries mood from activity.

STORY: a person opens the console, finds their agent in the sidebar by its face and bold name, reads what it did in work cards and threads, replies or steers, and when the agent uses its desktop a small live window appears that they can expand or take over. Management is a panel beside the conversation, never a separate app.

FIRST VIEWPORT: 1440×900. Left, the 260 px sidebar with Activity, Favorites, Agents (cube, name, activity), Rooms, Files, Skills, Platform. Middle, the selected agent's conversation: header with cube, name, activity and the Desktop and Details controls at the right; grouped messages with work cards and a "3 tasks · 2 done" thread line; the composer at the bottom with Cancel visible while a turn runs. Top right, over the conversation, the 320×200 floating live preview when an agent is on its desktop. Primary action: the composer's Send.

FORM: the category canon (Slack), user-pinned in the brief on 2026-11-01; no concept roll was run because the direction was pinned. Linear is the second reference for management density.

FINISH: unreviewed and undocumented is unfinished; this build ends with the finish review, the verdict, DESIGN.md, and every shipping raster carrying its provenance.

## Situation room (2026-11-04)

The user asked for one overview screen, so the thesis's refusal of a dashboard-first console still holds: the console opens on conversations, and this is one page you go to. It keeps the refusal's spirit by being a ruled ledger with one sentence on top, not tiles. No hero metrics, no cards, no sparklines; colour only for what wants attention, always with words.
