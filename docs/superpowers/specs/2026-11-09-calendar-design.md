# Calendar — design

Date: 2026-11-09. Status: draft, awaiting the user's approval.

## Intent

One place in the console where a person sees everything that happens on a timeline: their own calendar events and the work their agents are scheduled to do. It should look and feel like the macOS / iPhone Calendar: Month, Week and Day views, colored sources, and clicking an event opens a details card anchored where the pointer clicked. From the calendar the person can create, edit and delete both calendar events and routines.

Why: today calendar events live in each vendor's app and routine schedules are cron strings on each agent's Routines tab. Nobody can see "what happens tomorrow" in one look.

## Goals

- A **Calendar** page in the console (`apps/web`), Month / Week / Day views, today button, previous/next.
- Shows, merged on one timeline:
  1. **Calendar events** from calendar connections the viewing user manages (see Scope).
  2. **Upcoming routine runs**: scheduled (cron) routines of agents the user can manage, expanded into their occurrences in the visible range.
  3. **Past routine runs**: occurrences that already ran, with their outcome.
- **Create, edit, delete** calendar events and routines from the calendar.
- Clicking an item opens a **popover anchored to the click point** with its details and actions.
- Every time and date shown in the **user's time zone**.

## Non-goals (v1)

- Webhook-only or connection-event-triggered routines (they have no known time): never shown.
- Backups, dreaming, task deadlines and other platform schedules (possible later).
- Storing or syncing external events in PostgreSQL.
- Inviting attendees or responding to invitations from the calendar (agents already have `calendar_invite` / `calendar_respond`).
- Drag-and-drop rescheduling.
- Cal.com, Calendly and Zoom (booking/meeting tools, not calendars).

## Anti-goals

- Showing someone else's calendar to a user who does not manage that connection.
- A flood of blocks from high-frequency routines.
- Wrong times: an event shown at a UTC or browser time instead of the user's zone, or a recurring event missing / duplicated occurrences.
- A generic calendar library look; it should read as a native-quality Cubebot screen.

## Decisions made by the user

- **Scope:** only the user's own world — calendar connections the user manages, and routines of agents the user manages.
- **Noise:** routines running more often than hourly are collapsed: one chip per day ("every 5 min · Agent · Routine"), not a block per run.
- **Webhook routines:** not shown at all.
- **Recurring events:** must be expanded correctly — every occurrence, honoring exceptions (moved or cancelled occurrences) — for every supported connector.
- **Time zone:** always the user's time zone.
- **Editing:** v1 creates, edits and deletes events and routines.
- **Whose calendars:** calendar connections the user created (`connections.created_by`). This keeps the identity ADR's administrator-managed model; members who add no connections see only routines.
- **Routine time zones:** a routine stores an IANA time zone; its cron is evaluated in that zone, so 09:00 stays 09:00 across daylight-saving changes. Existing routines get `UTC` (no behavior change). Routines created from the calendar default to the user's profile zone. This amends the Routines ADR ("five-field cron in UTC") and reaches the routine endpoints, the scheduler's `next_at`, the Routines tab and the agents' own routine tools.
- **Recurring edits:** editing or deleting an occurrence of a repeating event asks "This event" or "All events", like Apple Calendar. "This event" writes a per-occurrence override (Google/Microsoft instance, CalDAV `RECURRENCE-ID` override or `EXDATE` in the master VEVENT). No "this and following".

## Constraints from the codebase

- Read `ARCHITECTURE.md`, `docs/adr/agents.md` (Routines), `docs/adr/identity.md` (native connectors, installation-managed connections) and `CONTRIBUTING.md`. Cyclomatic complexity ≤ 15 per `src/` function; `just check` must pass.
- Orchestrator rules stay in cohesive modules; a calendar module in the orchestrator, a `features/calendar` folder in the web app.
- **Routines** (`apps/orchestrator/src/work/routines*`): five-field cron in UTC, `Cron::after(t)` gives the next run. Upcoming runs are computed, not stored. Past runs come from routine occurrences. Existing create/update/delete endpoints are under `/api/agents/{agent}/routines`; reuse their rules (permissions, agent-managed routines, revision checks).
- **Connectors** (`modules/connectors`): the orchestrator already calls connector code with a connection's credential (`check`, `poll` in `connections/native/tend.rs`); the calendar calls `Connector::call` the same way. Tools at or below the connection's `level` only: a `read` connection's events are shown read-only.
- Current write support: Google `calendar_create_event`, `calendar_delete_event`; Microsoft `calendar_create_event` only; CalDAV `create_event`, `delete_event`. **None can update.** v1 adds update (and Microsoft delete, and CalDAV update) as ordinary connector tools, with fixture tests, so agents gain them too.
- **Time zone:** `identity/profiles.rs` stores `time_zone` (IANA, may be empty). Use it; fall back to the browser's zone when empty.
- **Ownership:** `connections.created_by` records who added a connection; per the identity ADR, connections are administrator-managed. Only calendar connections whose `created_by` is the viewer are in scope.
- Brand marks for connection sources come from `modules/brand/logos.ts`.

## How it works

### Backend (orchestrator)

- `GET /api/calendar?from&to` returns one list of items in the range, each with `kind` (`event` | `routine` | `run` | `routine_digest`), start/end (RFC 3339, UTC), all-day flag, title, source (connection or agent), color key, editable flag and kind-specific details.
- Events: for each calendar connection in scope, call the connector's list tool for the range, normalize, expand recurrences (below). No server cache in v1 (designer's call): the console caches per range with react-query. A failing connection does not fail the request: its status rides along so the UI shows "Google · sign in again".
- Routine runs: for each enabled cron routine of an agent the user created, iterate `Cron::after` (in the routine's zone) across the range. A routine that runs more often than hourly (more than 24 runs in a day) becomes one `routine_digest` per day instead.
- Writes: `POST/PATCH/DELETE /api/calendar/events…` routes to the connection's create/update/delete tool; routines reuse the existing routine endpoints (no duplicate API).
- Range limit (e.g. ≤ 6 weeks) so a request cannot ask for years of expansion.

### Recurring events

Server-side expansion differs (Google `singleEvents=true` and Microsoft `calendarView` expand; CalDAV only if the server supports `<C:expand>`). Guarantee: when a connector returns a master event with an RRULE, the orchestrator expands it itself, applying RDATE/EXDATE and RECURRENCE-ID overrides, in the event's own time zone (so DST shifts stay right), then converts to UTC. Tests cover daily/weekly/monthly/yearly, BYDAY, COUNT/UNTIL, exceptions, overrides, DST transitions and all-day events.

### Frontend (`apps/web/src/features/calendar`)

- Route in the console nav; Month, Week, Day; current-time line; all-day row.
- Sidebar: sources (each calendar connection, each agent) with color and show/hide toggles; remembered per user in local storage.
- Popover anchored at the click point (reuse the hover card positioning). Content by kind:
  - event: title, time, location, description, calendar; Edit, Delete (if editable), open in vendor app.
  - routine: agent, routine name, schedule in words ("Every weekday at 09:00"), next run; Run now, Edit, Delete.
  - run: outcome, duration, link to the conversation turn.
  - digest: cadence, number of runs that day, link to the routine.
- Creating: click an empty slot (or a "+" button) → choose Event (pick calendar) or Routine (pick agent, schedule builder plus raw cron).
- All formatting with `Intl.DateTimeFormat` using the user's zone.

## Calls made by the designer (not asked of the user)

- **Agents in scope:** agents the user created (`agents.created_by`), mirroring connections. An administrator does not see every agent's routines.
- **All calendars on an account:** every calendar a connection lists (`calendar_list`, `list_calendars`, Microsoft's calendars) is shown, with paging past the vendor page size. Sidebar toggles and colors are per calendar, grouped under their connection.
- **Event items** carry both a series id and an occurrence id so "This event / All events" can be routed.
- **Collapse threshold:** more than 24 runs in a day.

## Left to the builder

Exact visuals within the console's existing design language; caching durations; color assignment; endpoint naming; how the schedule builder phrases cron; recurrence library choice (a maintained Rust RRULE crate is fine if it passes the tests above, or a small in-house expander).
