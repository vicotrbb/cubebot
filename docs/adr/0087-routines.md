# ADR 0087: Routines

Status: Accepted

Date: 2026-11-01

ADRs 0046 to 0048 accept scheduled and event-triggered work, agent-managed triggers and per-trigger occurrence handling, and leave their mechanics open. This settles them with one shape.

Decision: A routine is a stored prompt, a shell command, or both, for one agent. The user selected this on 2026-11-01.

- **Occurrences.** A routine runs on demand by a person or by the agent, on a schedule, or by webhook. Each occurrence is a turn in the agent's permanent conversation with the routine's id and its origin (`person`, `agent`, `schedule` or `webhook`), inserted and woken like any chat message, so ADR 0005's lanes, retries and cancellation apply unchanged. The turn acts as the person who ran it, or, for the other origins, the person who created the routine or whose turn the agent created it in.
- **Coalesce.** A routine coalesces by default: while one of its turns is queued or waiting, a new occurrence makes no turn, so at most one runs and one is pending. Without coalescing every occurrence is its own turn. A coalesced occurrence leaves no record; ADR 0048's combined work with references to each occurrence is not built.
- **Schedules.** Five-field cron in UTC (minute, hour, day of month, month, day of week with 0 or 7 as Sunday; `*`, lists, ranges and steps; when both day fields are restricted either matches). A tick every 30 seconds claims due routines with `FOR UPDATE SKIP LOCKED`. A routine overdue after downtime runs once and its next time is computed from now, so missed occurrences are not replayed.
- **Webhooks.** `POST /api/hooks/{routine}` with a bearer secret a person creates or rotates in the console, shown once and stored as a SHA-256 digest; a wrong or missing secret is 401. The body, at most 8 KiB of UTF-8, is quoted into the message as a JSON string marked as untrusted data. An `Idempotency-Key` header seen in the last day returns the first answer without a second turn. Each routine accepts 60 deliveries a minute; more are 429. Agents cannot create webhook secrets, so ADR 0047's autonomy never exposes an endpoint.
- **Commands.** The turn delivery carries the command, which the agent runs as the tool user with bash before the model, journaled like a tool call; the model sees its result as untrusted program output. Without a prompt the turn answers with the result and makes no model call or provider request. A rerun after an interruption never runs the command again.
- **Management.** People manage routines with `conversation.send` on the agent and read them with `conversation.read`. The agent's `routine_create`, `routine_list`, `routine_update`, `routine_delete` and `routine_run` tools manage its own routines; it may change only routines it manages, and a person's edit makes a routine theirs, so the agent cannot undo a person's stop. A tool call's id makes a retried create or run idempotent. An agent holds at most 32 routines. Updates carry the expected revision. A disabled routine runs only by hand; its schedule and webhook stop. Deleting a routine cancels its queued and waiting turns and keeps the history.

Alternatives considered: A trigger table per source with an occurrence ledger, which records coalesced and rejected occurrences but adds a second work queue beside chat turns. Kubernetes CronJobs, which ADR 0046 rules out. Replaying every missed schedule after downtime, which floods an agent with stale work.

Open details: time zones other than UTC; payloads beyond the message, such as files; per-installation limits; occurrence retention; event sources other than webhooks.

Validation scenarios: The next time of a schedule; a coalescing routine keeps one pending turn; a wrong secret is 401; a replayed idempotency key makes no second turn; a command-only routine completes without a provider request.
