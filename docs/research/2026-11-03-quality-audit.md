# Quality audit, 2026-11-03

A whole-repository audit (ponytail-audit) run before the quality and cleaning pass. Four reviewers read the code by area: the orchestrator; the agent with `modules/protocol` and `modules/providers`; the sandbox VMM with `tooling/guest` and `deploy/`; the web console with `tests/system` and `tooling/quality`. Every case below comes from reading the code; none was reproduced against a running installation unless the line says so.

Load assumed: one installation, about 100 registered agents with 25 working at once (the first qualification target), a handful of people in the console.

Each finding ends with its outcome: **applied** (in the commit that added this report) or **not applied** with the reason.

## Orchestrator

1. **A routine someone else edits still ran as its creator** (`apps/orchestrator/src/work/routines.rs`). Scheduled and webhook runs act as `created_by`; a person's edit never changed it, so a member could rewrite an administrator's routine and keep it running with the administrator's authority after their own grant was revoked. **Applied:** a person's edit or secret rotation makes them the routine's actor; test added. Consequence: whoever edits a routine is who it runs as from then on.
2. **Every streamed text chunk is a ledger transaction and a permanent `turn_events` row** (`src/work/runtime.rs`). **Not applied:** deleting the rows after `done` contradicts the decision to retain durable data until explicitly deleted, and batching is the agent-side load change listed below (agent finding 9). Both wait for the scale qualification.
3. **The room coordinator used up its three attempts instantly** (`src/work/coordinator.rs`). A one-second provider 503 failed an unaddressed room message for good. **Applied:** a failed attempt waits before the next.
4. **Stalled transfers could lock out all file traffic** (`src/storage/volume.rs`, `src/api/files.rs`). Sixteen stalled peers held every slot for hours. **Applied** for uploads: each body read has a 60 s timeout and a stalled upload ends incomplete with nothing staged. **Not applied** for downloads: there the stall is a client that stops reading, so a timeout on the file read never fires; it needs a connection-level write timeout in the server, a separate change.
5. **Stopping an agent that never acknowledges its drain never finishes** (`src/agents/sandbox/docker.rs`, `src/agents/kubernetes/shutdown.rs`). **Not applied:** waiting for the acknowledgement is deliberate in the idle-lifecycle and maintenance decisions; a forced stop after a deadline is a behavior change that needs its own decision.
6. **The reconciliation tests drove a path production never runs** (`src/agents/kubernetes/reconcile.rs`). **Applied:** the private `converge` is gone; `reconcile` runs the production create/start/stop steps through shared helpers. Two tests changed with it: a missing bound workspace is now a conflict from `create` rather than an uncertain result, and a running pass reads the anchor and workspace before settling the pod.
7. **Production code only tests used** (`src/agents/kubernetes/client.rs` wrappers, `src/api/mod.rs` `database_router`). **Applied:** deleted; tests use the production functions.
8. **Login timing revealed which logins exist** (`src/identity/sessions.rs`). A missing login returned before the password hash check. **Applied:** a dummy verification runs for a missing account.
9. **Two concurrent `vault_put` calls with the same name failed with 503** (`src/connections/vault.rs`). **Applied:** the agent row is locked first.
10. **Every Docker reconcile pass ran three CLI calls per running agent** (`src/agents/sandbox/mod.rs`). **Applied:** the volume is created only when unknown.
11. **An unknown turn id returned 503 and logged an error** (`src/work/collaboration.rs`). **Applied:** answered as a missing turn.
12. **Token and digest helpers written twice** (identity and runtime sessions). **Applied:** one each.
13. **The administrator check written four times.** **Applied** for the two copies without a lock (`work/rooms.rs` now calls `identity::administrator`). **Not applied** for the others: one locks `FOR UPDATE` and one `FOR SHARE`, so they are different checks, not copies.
14. **`refused` and bearer parsing copied across API handlers.** **Applied:** one each in `api/mod.rs`.

## Web console and system tests

1. **Message box wedged after an unconfirmed send whose text was edited** (`apps/web/src/features/chat/Composer.tsx`, steering form in `routes/_app.agents.$agentId.thread.$turnId.tsx`). A lost reply keeps the request ID; editing the text and retrying sends the same ID with a different message, which the server refuses with 409 forever. The steering form also carried its ID across turns. **Applied:** editing the text drops the pending ID; the steering form is keyed by turn.
2. **Unread marks never clear** (`app/queries.ts` `markRead`, `app/events.ts`). The read PUT never updated the cached `read_sequence`, so a read room or idle agent stayed bold. **Applied:** `markRead` updates the cached list.
3. **Saving one About section discarded unsaved edits in the others** (`routes/_app.agents.$agentId.about.tsx`). The panel was keyed by revision and remounted on save. **Applied:** keyed by agent only.
4. **Sign out did nothing once the session had expired, and was written twice** (`features/shell/Sidebar.tsx`, `routes/_app.you.tsx`). Logout returns 401 without a session; the throw skipped clearing the cache and the redirect. **Applied:** one `signOut()` that always clears and redirects.
5. **Rooms could show an older delivery state and jumped to the bottom on every update** (`features/rooms/Room.tsx`). A history reload during reconnect could overwrite "replied" with "working". **Applied:** a finished delivery never goes back; the scroll follows only when pinned to the bottom, as in the conversation view.
6. **One bad screen tile froze the live desktop for good** (`features/desktop/useDesktopSocket.ts`). The paint promise chain stayed rejected. **Applied:** each link catches its own failure.
7. **Files listed the wrong folder after quick navigation; old errors never cleared** (`features/files/Files.tsx`). **Applied:** late responses are ignored and navigation clears the error; the misleading retry comment is fixed.
8. **A closed installation stream polled only the agent list** (`app/events.ts`). Room markers and an expired session were never refreshed. **Applied:** the fallback also refreshes rooms and the session.
9. **Every open agent screen polled for a login hand-off every 3 s** (`features/desktop/LoginBanner.tsx`), even with none pending. **Applied:** polls only while `pending_login` is set.
10. **Stopgaps from before the API contract** (`api/types.ts`, `app/queries.ts`, `api/admin.ts`). The generated schema now carries every route and field; the 404 fallback for `/api/installation` claimed computer use and the terminal were enabled, and `admin.ts` hand-typed about 200 lines the schema defines. **Applied:** fallbacks deleted, types are schema aliases, calls go through the generated client.
11. **Dead exports, unused CSS and small duplicates** (`About.tsx`, `Conversation.tsx`, `useConversation.ts`, `Room.tsx`, `Sidebar.tsx`, CSS). **Applied:** deleted or merged.
12. **`tests/system/docker_*.py` repeated provider setup, `agent()` and `grant()`** in up to ten scripts. **Applied:** one copy each in `docker_sandbox.py`.

13. **Duplicate OpenAPI operation names** (`apps/web/src/api/openapi.json` from the orchestrator). `list`, `read` and `events` are each used by several routes, so the generated types keep only the first: the invitations list, access events and both read-cursor routes were typed wrongly. Found while applying finding 10; the hand-written suspend call had also sent `account_id` where the server reads `id`, fixed by moving to the generated client. **Applied:** unique operation names, regenerated client, casts removed.

`knip` also listed `public/sw.js`, the three Bun test files and the `.mjs` browser harnesses as unused files. They are not: `sw.js` is registered from a string in the theme boot script, the tests run from the `justfile`, and the harnesses run from the Python system tests.

## Agent runtime, protocol and providers

1. **Tool calls with more than 16 KiB of arguments failed the whole turn** (`modules/providers/src/lib.rs` `call`). A `write_file` with a 17 KB file ended the turn as `provider_invalid_response`. **Applied:** the bound is 1 MiB, inside the stream limit.
2. **A tool result over 64 KiB failed the turn and leaked journal records** (`apps/agent/src/journal`, `engine/attempts.rs`). The journal refuses records over its payload limit; the unfinished call was never released, so after about 4,096 records every tool call failed until restart. **Applied:** the journaled copy is truncated to fit; the ledger still receives the full event.
3. **Dreaming always failed on Anthropic** (`engine/dream.rs`). The dream request passed no output limit, which the Anthropic adapter requires. **Applied:** the dream asks the provider for its limits as a turn does; loopback test added.
4. **One failed send could stop an agent from pulling work** (`engine/mod.rs` `forward`, `idle`). A finished turn stayed in `active` when its `Done` failed to send. **Applied:** removed before sending; finished tasks are dropped when idle.
5. **The agent never noticed a dead control connection** (`engine/mod.rs`). Without a FIN or RST it waited forever. **Applied:** 15 s of silence, pings included, reconnects, matching the orchestrator's timeout.
6. **A delegated task could drive the main browser session** (`engine/turn.rs`, `tools/browser.rs`), against the main/task session decision. **Applied:** task lanes are refused the `main` session.
7. **A stdio MCP server could exhaust agent memory** (`tools/mcp/stdio.rs`) with a line that never ends. **Applied:** lines are bounded like the HTTP transport.
8. **The computer tool kept acting after cancellation** (`tools/computer.rs`), and the action list had no bound. **Applied:** a cancel flag checked between steps and a cap on actions.
9. **Every streamed text chunk is its own ledger transaction** (`engine/turn.rs` with `apps/orchestrator/src/work/runtime.rs`). About 600 transactions a second at 25 streaming agents. **Not applied:** a load change, not a defect at today's measured use; it needs the scale qualification to size the batching, and STATUS already lists delivery load at scale as unmeasured.
10. **A refused connection to the provider was never retried** (`modules/providers/src/lib.rs`). **Applied:** connect errors are transient.
11. **Every failed OpenAI response was retried** (`modules/providers/src/openai.rs`), including invalid requests. **Applied:** only rate-limit and server errors are transient, as in the Anthropic adapter.
12. **The strict tool-schema builder was copied seven times** across agent tools. **Applied:** one `Tool::strict` constructor.
13. **Hand-rolled base64** (`desktop/png.rs`) while `base64` is already in the lockfile. **Applied:** uses the crate; CRC32, Adler-32 and deflate stay (nothing installed provides them).
14. **Small helpers in two or three copies** (SSE parsing, bounded body reads, tool-user `uid:gid` parsing, `Refused` decoding, pending-reply maps). **Applied** for the tool-user parse, refusal decoding and pending-reply map; the provider SSE reader keeps a scan offset instead of rescanning. **Not applied** for sharing SSE parsing and bounded reads between `providers` and the agent's MCP client: each copy has its own error type, and the MCP stream answers pings mid-stream, so one copy would mean exporting SSE internals from `providers` for one caller.
15. **The post-crash "uncertain" result could reuse a sequence the ledger already holds** (`engine/attempts.rs`), so the call never got an outcome. **Applied.**

## Sandbox VMM, guest build and deploy

No guest-to-host memory escape was found: every guest access goes through the bounds-checked `GuestMemory::offset`, descriptor chains are capped at the queue size, and `unsafe` stays in `src/kvm`.

1. **The chart regenerates the credential key and database password outside `helm upgrade`** (`deploy/chart/templates/secrets.yaml`). Under Argo CD, Flux or `helm template | kubectl apply`, `lookup` returns nothing, so each sync renders a new key and password; stored credentials can no longer be decrypted. **Not applied:** the fix needs a new chart value for an existing secret, and this pass keeps configuration keys unchanged. Until then, install the chart with `helm install`/`helm upgrade` only. This is the highest-priority follow-up.
2. **A compromised guest could grow sandbox memory without limit** (`apps/sandbox/src/kvm/virtio/vsock.rs` reply queue and spare-stream channel), and the Docker `kvm` sandbox had no memory or process limit. **Applied:** both queues are bounded; Docker gets the pod's limits.
3. **The device plugin capped `kvm` agents at 8 per node** (`deploy/chart/templates/device-plugin.yaml`), undocumented. **Applied:** 110, the Kubernetes default pod limit per node.
4. **Sandbox NetworkPolicy broke DNS with NodeLocal DNSCache** (`deploy/chart/templates/networkpolicy.yaml`). **Applied:** port 53 is allowed to the addresses in `sandbox.dns`.
5. **The vsock control port accepts a stream from any guest port** (`virtio/vsock.rs`). No trigger exists in today's init. **Not applied:** it needs the guest to bind a reserved port too, and the KVM boot cannot be checked on the machine that ran this pass; a broken change would stop every `kvm` agent.
6. **A vCPU error discarded the exit status the guest had already reported** (`src/kvm/mod.rs`). **Applied.**
7. **Projected files were marked sent although not delivered** (`src/kvm/mod.rs` `refresh`). A rotated token could be skipped. **Applied.**
8. **Unchecked additions on guest addresses** (`virtio/queue.rs`) panicked in debug builds while holding the device lock. **Applied:** checked.
9. **The desktop socket path and `invalid` were each defined twice** (`main.rs`/`bridge.rs`, `boot.rs`/`memory.rs`). **Applied.**
10. **`deploy/install.sh` ran partially when a `curl | sh` download was cut off.** **Applied:** the body is one function called on the last line.
11. **The guest build reused a half-extracted kernel tree** (`tooling/guest/build.py`). **Applied:** extracts beside and renames.

## Verdict

The highest-risk defects (a routine edit that kept an administrator's authority, turns failing on large tool arguments or results, Anthropic dreaming, an agent that stops pulling work after a failed send or a silent connection) are fixed with tests. Open, in priority order: the chart's regenerated secrets under GitOps (sandbox 1), the download-side transfer timeout (orchestrator 4), text-delta load at scale (orchestrator 2, agent 9), the drain deadline (orchestrator 5) and the vsock control-port check (sandbox 5).

Not read in depth: most Kubernetes registry and recovery modules, `operations/`, the OIDC HTTP client, several console screens and the `.mjs` browser harnesses. The KVM code was compiled and tested on x86_64 Linux in a container, not booted under KVM; nothing in this report was exercised on a cluster.
