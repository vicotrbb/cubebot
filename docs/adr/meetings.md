# Meetings

Cubebot's own meeting tool: links anyone can open, agents as participants, a transcript and summary in each agent's chat. This document records the decisions for it. Joining meetings on other platforms (Meet, Teams, Zoom) is a separate area and is not decided here; it will share the settings and the transcript model below.

## Cubebot Meetings

**Status: Proposed 2026-11-15.** Nothing here is accepted yet, and one choice below needs the user: the SFU, which adds a service. The orchestrator-side parts (ownership, links, guest tokens, transcript, settings) ship first in phase 1 and do not depend on that choice.

> **Decision needed from the user:** run LiveKit, self-hosted, as a new Deployment in the installation's namespace (recommended), or take one of the alternatives below. Until you choose, no media code is written and no service is added to the chart.

### Decision (proposed)

1. **The orchestrator owns meetings.** Meetings, participants, public links, invitations, permissions, transcript and the join tokens guests hold are rows in PostgreSQL, behind the orchestrator's authorization like every other domain. The browser never trusts the media server for who may join.
2. **An SFU carries the media.** WebRTC with a selective forwarding unit; a mesh does not hold past a few people. The SFU is a separate, replaceable service that only forwards audio and video for rooms the orchestrator names and tokens the orchestrator mints. It holds no accounts, no history and no agent state.
3. **The agent bridge lives in the orchestrator**, one per agent in a meeting. It joins the SFU as a participant, turns the room's audio into text with the existing speech module, hands the agent text, and speaks the agent's reply as an audio track. Agents still see text only, and the sandbox gets no media code ([Voice calls over the chat pipeline](console.md#voice-calls-over-the-chat-pipeline)). An agent's screen share is the agent desktop's frames published from the bridge.
4. **Guests are not accounts.** See the security model.
5. **Every agent in a meeting gets a transcript and summary** as one Meeting message in its permanent conversation, as a call produces a Call message. Transcript segments (speaker, time, text) are stored once, in PostgreSQL; the summary is made when the meeting ends.

### The SFU: options

| Option | For | Against |
| --- | --- | --- |
| **LiveKit**, self-hosted (recommended) | Rooms, scoped access tokens, simulcast, screen share, recording and server and client SDKs ready-made; tokens fit decision 1 | A new service in the Helm chart; UDP media ports and a TURN path in Kubernetes and home networks; a Go dependency in a Rust-only stack |
| mediasoup | Mature and flexible; a library, so its signalling and rooms are ours to shape | Node and C++ workers to run; rooms, tokens and recording are ours to write |
| In-house Rust (`str0m` or `webrtc-rs`) | Keeps the repository Rust-only; no extra service | Months of work, a worse result, and a media stack to secure and keep current |

These comparisons come from desk research, not a prototype. Before accepting, a spike must check: LiveKit's licence terms for self-hosting as part of an Apache-2.0 project; the maturity of its Rust SDK, which the agent bridge needs; whether the Helm chart can expose UDP (and TURN over 443 as the fallback) on the clusters Cubebot targets; and WebRTC behaviour in the iOS Safari and installed PWA, which is already fragile.

Choosing LiveKit means the chart gains one workload next to the orchestrator and frontend, still in the one namespace ([One namespace for the entire installation](packaging.md#one-namespace-for-the-entire-installation)). The orchestrator stays the only service that touches PostgreSQL and the only one holding the SFU's API secret, which lives in the installation's existing secret handling.

### Guest security model

- **A link is a bearer secret.** 256 random bits; only its SHA-256 digest is stored; shown once at creation. It can expire (limiting when new guests may join, not guests already in), be revoked in one call, and open a lobby. Whether new links use a lobby follows the installation setting, on by default.
- **A guest holds a join token, never a session.** Joining with a link and a name gives a short-lived token bound to one participant in one meeting. Join tokens live in their own table, and nothing that authenticates a person reads it: the console API does not accept a guest token, and no console credential is issued to a guest. Guest endpoints live under `/api/meeting-guests` and accept nothing else.
- **Two scopes.** A `lobby` token (30 minutes) can only ask whether the host has let the guest in. A `media` token (5 minutes) can join the call and is issued only to an admitted guest; the guest refreshes it while they stay. The media side calls the orchestrator to verify a token before it lets anyone in, and the orchestrator mints the SFU's own room token from a verified `media` token.
- **Standing is checked live.** A token works only while its guest is waiting or admitted, its link is not revoked, and the meeting is open. Revoking a link therefore ends the access of every guest who came in through it; removing a guest ends theirs; ending the meeting ends everyone's.
- **No oracle.** An unknown, revoked or expired link, and an ended meeting, answer alike.
- **Abuse limits.** A meeting takes at most 100 guests waiting or admitted at once. Per-address join rate limits come with the public deployment work (open details).
- **Consent.** The installation setting (on by default) tells everyone in a meeting that agents may listen and a transcript is kept. A meeting copies its settings at creation.
- **Only the creator manages a meeting.** Administrators get no extra reach into others' meetings, as with calendars. Inviting an agent needs `conversation.send` on it, as a call does.

### Settings

A Meetings tab in the console's settings holds the installation defaults: lobby on, consent notice on, and agents joining in `on_mention` mode. Later phases add the STT and TTS provider and voice for meetings, recording on or off, and the invitation email connection. The same tab will hold the settings for agents joining meetings elsewhere.

### Agent modes

An agent takes part as `listen_only`, `on_mention` or `proactive`. Phase 1 stores the choice. The speaking policy that acts on it (one floor, humans win, addressed agent first, cooldowns, a cheap model to choose among several) is phase 2 and needs a replay harness before `proactive` is offered.

### Phases

1. **Core (first change).** This decision, the data model, links, guest tokens, lobby, agent invitations, transcript storage and the settings; no media.
2. **Audio room, one agent.** The SFU, the `/meetings` route, the agent bridge, transcript and the Meeting message with its summary in the agent's chat.
3. **Several agents** and the speaking policy.
4. **Video and human screen share.**
5. **Agent screen share.**
6. **Email invitations** through a chosen mail connection, calendar events, and recording.

### Consequences

- A new meeting is a few rows, no process. Phase 1 adds no service, no dependency and no port.
- The SFU choice can change without touching meetings, links, tokens or the transcript, because the orchestrator owns them and the SFU sees only tokens it minted.
- Public guests are the first unauthenticated write path on the orchestrator. They reach only `/api/meeting-guests/join` and `/refresh`, each bounded and audited (`meeting.join`).
- The 100-agent target is untested for meetings; cap the agents in a meeting when the bridge arrives.

### Open details

SFU choice; passcodes on links; per-address join rate limits and CAPTCHA; how a guest leaves and presence; recording and its retention; how long an ended meeting's guests, join tokens and transcript are kept; the cap on agents per meeting; summary prompts and which model writes them; whether invited console users appear as participants or only as hosts; the Meetings tab's design (Pixel).

### Validation scenarios

A guest without a lobby gets a `media` token, and one with a lobby gets a `lobby` token that cannot join until admitted; a revoked or expired link admits no one, and revoking it ends the access of guests already in; a guest token is refused by every console route; a token for one meeting never verifies for another; the media side refuses a `lobby` token; an ended meeting accepts no join and records no segment; once media exists, a bridge-spoken reply reaches the room within a latency budget set by the spike, and a human speaking stops agent audio at once.
