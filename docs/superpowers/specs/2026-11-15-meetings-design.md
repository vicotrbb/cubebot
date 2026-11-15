# Meetings: design

Date: 2026-11-15. Status: the user's decisions are recorded below; the rest is the designer's and open to overrule.

## Intent

Cubebot gets its own meeting tool. A person starts an instant meeting or schedules one, invites people and agents, and shares a link that anyone can use. Everyone in the meeting has voice, video and screen sharing. Agents take part by voice: they hear the meeting, answer when they are addressed, and can share their own desktop. Each agent that attends finds the meeting in its own chat as one Meeting message holding the transcript and a summary, as a call does today.

## Decisions made by the user

- **Scope.** Meetings that Cubebot owns, with video, voice and screen sharing, public links, instant links, invitations by email through a connection the person selects, agents as participants, agent screen sharing, invitations while the meeting runs, meetings on the Calendar and in the New menu, and a Platform › Meetings settings tab. Agents joining meetings in other applications is a later project.
- **Speech.** Agents' STT and TTS follow the platform's speech settings, exactly as calls do.
- **Media transport.** A WebSocket relay through the orchestrator. Browsers encode video with WebCodecs; nothing else is added: no WebRTC, no TURN, no UDP.
- **Guests and agents.** A guest (someone who joined by link without signing in) may talk to agents, but a reply a guest's words caused runs with no tools, enforced by the agent runtime. Signed-in members get the agent's full abilities, as in chat.
- **Admission.** Each meeting has an "Anyone with the link joins directly" switch, off by default; while off, guests ask to join and a signed-in participant admits them. Signed-in members always join directly.
- **Who creates.** Every member creates meetings and invites people. Agents can be invited only by someone who may send to them (`conversation.send`). Email goes through the inviter's own email connections. The New menu shows to everyone, with only the items each person may use.
- **Public entry.** The public entrypoints are protected completely, and Kubernetes installs may add a second, public ingress that carries only them.
- **UI.** Desktop, mobile browsers and the installed PWA are all first-class.

## Non-goals

- Agents joining Google Meet, Zoom or Teams (later project; the settings tab leaves room for it).
- Recording audio or video. The transcript is the record.
- In-meeting text chat, reactions, breakout rooms, virtual backgrounds.
- Dial-in by phone.
- More than one active meeting session per meeting at once.

## Anti-goals

- A guest gaining anything beyond the one meeting: no other API, no agent tools, no files, no names of other agents or people outside the meeting.
- An agent that talks over people, answers every sentence, or loops with another agent.
- A meeting that stalls for everyone because one participant's connection is slow.
- Guessable or long-lived links that outlive their meeting's intent.

## Architecture

```
browser (member or guest)
  ├─ mic  → AudioWorklet 16 kHz PCM16 ─┐
  ├─ cam  → WebCodecs VideoEncoder ────┼─ WSS /api/meetings/{id}/live      (member, cookie)
  └─ screen → WebCodecs VideoEncoder ──┘   WSS /api/public/meetings/live   (guest, link token)
                                             │
orchestrator: meetings module, one Room per active meeting (in memory, one active orchestrator)
  ├─ relay: forwards each sender's audio and video to every other participant, per-receiver bounded queues
  ├─ hearing: one speech Listener (Silero + STT) per speaking person → utterances → transcript
  ├─ floor: decides which agent answers, one agent speaks at a time, people interrupt agents
  ├─ voices: agent reply text_delta → sentences → TTS → audio frames to everyone
  └─ screens: an agent's shared desktop, polled once, tiles fanned out to everyone
PostgreSQL: meetings, invitations, utterances; chat_turns.meeting_id links agent turns
```

The orchestrator already relays call audio and desktop tiles, and only one orchestrator is active, so a meeting's live state is an in-memory room keyed by meeting id. Durable facts (the meeting, its invitations, every utterance, every agent turn) are rows, written as they happen.

### Wire protocol (one WebSocket per participant)

Binary frames from the browser start with one type byte:

| byte | payload                                                                      |
| ---- | ---------------------------------------------------------------------------- |
| `1`  | microphone, 16 kHz mono PCM16 little-endian, as calls send                   |
| `2`  | camera chunk: flags (bit 0 key), u32 timestamp in ms, then the encoded chunk |
| `3`  | screen chunk, same layout                                                    |

Binary frames to the browser start with the type byte and a u16 source number (the participant's slot), then the same payload; agent audio carries a 4-byte sample rate before its PCM, as calls do.

JSON text frames carry everything else: `roster` (participants, their slot, kind, name, avatar, microphone, camera, screen, speaking, hand), `track` (a sender's codec, width and height, so receivers configure a decoder), `keyframe` (the server asks a sender for one when a receiver joins or dropped frames), `caption` (a partial or final line), `agent` (an agent's state: listening, thinking, speaking), `desktop` (an agent's screen tiles, as the desktop page receives them), `lobby` (people waiting, for admitted members), `admitted`, `denied`, `removed`, `ended`, `notice`. Browser text frames: `state` (microphone, camera, screen, hand), `admit`, `deny`, `remove`, `lock`, `share_agent_screen`, `end`.

Video: each sender picks the first codec its `VideoEncoder` supports from VP8, H.264 baseline and VP9; camera at up to 640x360 and 24 fps around 500 kbps, screen at up to 1920x1080 and 8 fps around 1.5 Mbps, a keyframe every 4 s and on request. Receivers decode with `VideoDecoder` onto a canvas. A browser without WebCodecs still joins with audio and sees agent screens; its tiles show the person's avatar.

Back-pressure: each receiver has a bounded queue. Audio frames drop oldest first; when a video frame would not fit, that sender's video to that receiver is dropped until the next keyframe, which the server asks for. One slow receiver never slows anyone else. A sender is limited to 4 Mbit/s and 200 frames per second; frames past either are dropped.

### Agents in a meeting

- **Joining.** Invited agents join as soon as the first person does, and an agent invited during the meeting joins at once. Agents need the installation switch "Agents may join meetings". A retired agent never joins.
- **Hearing.** Every person has their own listener, so each utterance has a known speaker. People's speech is transcribed with the installation STT provider; a key-based provider uses a key an agent in the meeting may use, and with no such key the embedded recognizer is used.
- **Deciding who speaks (the floor).**
  1. An utterance naming an agent (its name or first name, case-insensitive, as a whole word) goes to that agent; several named agents answer in the order named.
  2. Otherwise, when the meeting's agent mode is "When relevant" (the default), a tool-free model call (the room coordinator's provider and helper) reads the last few lines and answers which agent, if any, should speak. Its instructions are conservative: answer only when an agent is addressed, a question is put to the group that an agent is suited to answer, or the person is continuing an exchange with that agent. "Only when named" skips this call.
  3. The decision waits 700 ms after the utterance ends; if the same person speaks again, the utterances merge and the wait restarts, so an agent never answers half a sentence.
  4. One agent speaks at a time; others queue. When any person starts speaking, the speaking agent stops at once (barge-in, as in calls) and its queued replies are dropped; its turn is not cancelled and its full reply stays in the transcript.
  5. An agent's spoken words are transcribed as its own lines. A line that names another agent may hand the floor to it, at most twice in a row before a person speaks again, so agents cannot loop.
- **Turns.** An answer is an ordinary `chat_turns` row in the agent's conversation with `meeting_id` set, sent through the same path as chat. Its message carries the meeting's lines since the agent's previous turn there (at most 40 lines), marked as context and not instructions, then the line it answers. The turn's instructions add that it is in a meeting, who is present, and to answer briefly in plain speech, like the call instruction.
- **Whose request.** A member's words run as that member when they may send to the agent. A guest's words, or a member's who may not send, run as the meeting's owner with `talk_only`, a new turn flag under which the agent offers no tools at all.
- **Screen sharing.** In a meeting turn whose agent has a desktop, the agent gets a `meeting_screen` tool (`share: true|false`), and a participant who may view the agent's desktop can ask from its tile. Needs "Agents may share their screen". The orchestrator polls that desktop once per meeting and sends its tiles to everyone; a participant who joins later triggers a fresh full frame for everyone.
- **In each agent's chat.** Each agent that attended gets one Meeting message in its conversation, as a call does: title, time, duration, the people and agents present, the transcript and the summary, with the agent's own turns inside. History and the event stream return it in sequence order through a `meeting` field on `Turn`, as `call` is today.
- **Summary.** When a meeting ends, one tool-free model call through the owner's coordinator provider writes the summary from the transcript. It is stored on the meeting and given to each attending agent as a completed, non-answered turn (`Meeting "X" ended. Summary: ...`), so the agent's later context includes it with no special handling. Without a provider, the summary says it is unavailable and the transcript remains.

### Meetings, invitations and links

- **A meeting** has a title, an optional description, an owner, an optional schedule (start and end) or none for an instant meeting, a link, an agent mode, the admission switch, a lock, and its state: `scheduled`, `live`, `ended`. It is live while anyone is in it, and ends when the owner ends it for everyone or five minutes after the last person leaves.
- **Invitations** name a member, an agent or an email address. Invitations can be added before or during the meeting. A member invited sees the meeting on their Calendar and on the Meetings page and may join it signed in. An email address receives the public link.
- **Email.** The inviter picks one of their own email connections that offers a send tool (Gmail `gmail_send`, Microsoft `mail_send`, IMAP/SMTP `send`). The body is plain text: title, time in the inviter's zone, description, the link, and "this meeting is transcribed". Each send's outcome shows on the invitation; a failure never blocks the meeting.
- **The link** is `<public origin>/m/<token>`, a 32-byte random token in base64url. Only participants of the meeting see it. The owner can turn the link off or replace it, which invalidates the old one. A link to an ended meeting says so and nothing more.
- **Who sees what.** A meeting is visible to its owner, its invited members, and administrators through the audit log only. Members see the transcript and summary of meetings they were invited to or attended.

### Public entrypoints and their protection

- Public routes live under `/api/public/meetings/` and the page `/m/{token}`; nothing else accepts a guest. They read no cookie, so a member's session never turns a guest request into a member's.
- `GET /api/public/meetings/{token}` answers only the title, the host's display name, whether the meeting is live, ended or locked, and whether guests wait in a lobby. An unknown token and a disabled link give the same 404.
- Guests join through the live WebSocket with the token; the first frame names them (1 to 40 characters, control characters stripped, shown as "Name (guest)"). Guests are refused when the installation turns public links off, the link is off, the meeting is locked, ended, or full (installation maximum, default 12 participants, agents included), or the lobby already holds 20.
- Every public request is rate limited in the orchestrator, per client address (the first `X-Forwarded-For` hop only when `CUBEBOT_TRUSTED_PROXY_HOPS` says a proxy is in front, else the socket address) and in total: lookups 30 a minute per address, joins 10 a minute per address, and lookups of unknown tokens 200 a minute in total, after which unknown tokens are refused for a minute.
- The live socket checks `Origin` against the console origin and the public meetings origin. Frame size is capped at 256 KiB; the WebSocket closes after 15 s without a frame and a guest is dropped when the meeting ends, they are removed, or the link changes.
- A guest learns only the names, avatars and media of the meeting's participants. An agent's avatar and name are shown; its id is not.
- Every admission, denial, removal, lock, link change and guest join is audited.

**Kubernetes.** `publicOrigin` stays the console. A new `meetings.publicIngress` value adds a second Ingress (host, class, TLS secret, annotations) that routes only `/m/`, the web app's static assets and `/api/public/meetings/` and sets `CUBEBOT_MEETINGS_ORIGIN`, so links and the Origin check use it. The console ingress may then stay private. The chart's README section shows ingress-nginx rate-limit and body-size annotations, and Gateway API gets the same split through `meetings.publicHttpRoute`. Compose with a domain needs nothing extra: the public origin defaults to the console origin.

### Settings: Platform › Meetings

Administrators only, one row in `meeting_settings`:

- Agents may join meetings (on).
- Agents may share their screen in meetings (on).
- Guests may join by link (on).
- Largest meeting, participants including agents (12, from 2 to 24).
- Default agent mode for new meetings: When relevant / Only when named.
- Public address, read-only: the origin links use, and how to set it.

### Console

- **New menu** for everyone: New meeting (everyone), Agent, Team, Room (administrators). New meeting opens a sheet with two paths: _Start now_ (an instant meeting, straight to the green room) and _Schedule_ (title, date, start and end, description, people, agents, email addresses, email connection, admission, agent mode).
- **Meetings page** in the sidebar: Live now, Upcoming, Past; each past meeting opens its summary and transcript.
- **Green room** (before joining): camera preview, microphone level, device pickers, mute and camera toggles, display name for guests, the transcription notice, Join. Guests waiting in the lobby see a calm waiting state.
- **The room:** an adaptive grid of tiles (people with video or avatar, agents as their cube avatar that animates while listening, thinking and speaking); a presented screen takes the stage with a filmstrip; live captions; a bottom bar with microphone, camera, present, captions, raise hand, people, leave; a side panel (People with invite, lobby and link; Transcript) that becomes a sheet on phones. Speaking participants get a ring. Keyboard: M mic, V camera, C captions, H hand.
- **Calendar:** meetings the person owns or is invited to appear as their own source "Meetings", with Join in the popover, and the calendar's New dialog gains Meeting.
- **Agent chat:** the Meeting message as described above.

## Data model (migration 0080)

- `meeting_settings` singleton: `agents_join`, `agent_screens`, `guests`, `max_participants`, `agent_mode`.
- `meetings`: `id`, `title`, `description`, `owner_id`, `starts_at`, `ends_at`, `link_token` (unique, null when off), `open_join`, `locked`, `agent_mode`, `started_at`, `ended_at`, `summary`, `created_at`.
- `meeting_invitations`: `id`, `meeting_id`, exactly one of `account_id` / `agent_id` / `email`, `invited_by`, `created_at`, `email_connection_id`, `emailed_at`, `email_error`.
- `meeting_attendance`: who joined and left, members by account, guests by name, agents by id, with the agent's `sequence` from `chat_turns`' sequence so its Meeting message sorts in its conversation.
- `meeting_utterances`: `meeting_id`, `sequence`, speaker (account, agent, or guest name), `text`, `at`.
- `chat_turns.meeting_id`.

## Testing

- Rust unit tests: frame parsing and limits, floor decisions (naming, merge window, barge-in, agent hand-off cap), rate limiter, guest name sanitizing, link token, back-pressure.
- Integration tests: a meeting's agent turns are ordinary chat turns grouped into one Meeting message; a guest's words make a `talk_only` turn as the owner; a guest is refused when the link is off, locked, full, ended or the lobby is full; the public lookup answers nothing beyond its fields; settings are administrator-only; invitations of agents need `conversation.send`.
- Agent unit test: a `talk_only` turn offers no tools.
- Web unit tests: the room reducer and frame codec.
- Browser check: two Chromium pages with fake devices join one meeting, see each other's video, and a stand-in agent answers a named question with audio; desktop and phone screenshots of every screen.
