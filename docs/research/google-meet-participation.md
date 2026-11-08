# Agents in Google Meet: research

Researched 2026-11-07, after a prototype of agents joining Meet as guests (built on an unmerged branch, then removed) hit Meet's refusal of automated guest joins, which began in late October 2026. [What the prototype built and measured](#what-the-prototype-built-and-measured) records that work so it need not be repeated. The question: how can an agent listen to a Meet call (live audio or captions), and ideally speak, without a Google account per agent and without evading Google's bot detection? Evasion, fingerprinting, CAPTCHA solving and proxy rotation are out of scope, and sources about them were skipped. Claims without a primary source are marked _unverified_.

## The short version

- **The Meet Media API is gone.** Google deprecated it on 2026-10-28. Sign-ups are closed and its endpoints shut down on 2027-01-15. It was the only official way to get live Meet audio, and it was receive-only.
- **No official API lets a non-human participant join a Meet call.** The REST API reads artifacts, and add-ons get no audio. Service accounts only call APIs.
- **Google's refusal of automated guest joins is undocumented.** No Workspace Updates post, release note or help page announces it. Vexa's production logs show it starting around 2026-10-29. The documented change is older: from March 2026, hosts see knocks from risky connections in a separate queue whose default action is _Deny_. Google's Terms forbid "bypassing our systems or protective measures", so Cubebot must treat a refusal as final.
- **Without new Google accounts, the dependable live paths need a human.** Either the person is in the call and their own browser forwards the meeting to the agent, or the meeting belongs to a Workspace organization with dial-in, and the agent calls in through a phone provider. Meetings hosted by a consumer @gmail account have no dial-in, no transcripts and no Meet REST artifacts that we could confirm.
- **One shared Workspace account** opens signed-in joins, skipping the knock when the account is invited, plus dial-in numbers, Internal OAuth and transcripts. It costs one seat, gives every agent the same Google identity, and still needs a human to sign in once.

## 1. Meet Media API (real-time media)

**Status: discontinued.** The Meet release notes for 2026-10-28 say: "we have decided to wind down the Google Meet Media API and permanently turn down its endpoints on January 15, 2027 … For developers looking to build intelligent meeting workflows … we recommend transitioning to the Meet REST API." The Meet landing page says "New signups are now closed." Every Media API guide now returns 404, and `googleworkspace/meet-media-api-samples` was archived the same day with a deprecation notice. Facts below come from the reference clients and a 2026-09-30 archive of the overview, for the record only.

- **Enrollment:** "the Google Cloud project, OAuth principal, and all participants in the conference must be enrolled in the Developer Preview Program." That rules out arbitrary meetings even before the shutdown.
- **Scope:** `meetings.conference.media.readonly` (from the TypeScript sample). Restricted scopes stored or sent off the device trigger a security assessment, according to the Meet scopes page.
- **Consent:** someone allowed to consent for the meeting must be in the call. For Workspace meetings that is a member of the owning organization. "For meetings organized by Gmail accounts, the initiator must be in the meeting to provide consent." The host must allow the app in Calendar settings, anyone can stop it, and it is refused when minors, encryption or a watermark are present.
- **Direction and limits:** receive-only. Both clients create audio and video transceivers as `recvonly`, and "Meet only supports 3 audio virtual ssrcs". Audio arrives as three virtual streams of the most relevant speakers, each tagged with a CSRC.
- **Clients:** C++, built on WebRTC with a Bazel/Ubuntu 24.04 Dockerfile, so it ran headless on Linux; and TypeScript for the browser.
- **Who used it:** Read AI announced its "botless" Meet integration on it on 2026-03-26.

| Feasibility | Listen        | Speak | Setup | Cost | Risk                 |
| ----------- | ------------- | ----- | ----- | ---- | -------------------- |
| None        | Was yes, live | No    | n/a   | n/a  | Shut down 2027-01-15 |

## 2. Meet REST API (artifacts)

This API is generally available. It covers spaces, conference records, participants, recordings, transcripts, transcript entries and smart notes, and since 2026-09-11 it can also manage members and co-hosts.

- **Latency:** artifacts are "usually ready to be fetched soon after a conference ends." Transcript entries carry speaker, text and timestamps and are kept for 30 days. The Workspace Events API reports `transcript.v2.started/ended/fileGenerated` and participant joins and leaves through Pub/Sub. No transcript-entry event exists. _Unverified:_ whether `transcripts.entries.list` returns entries while the meeting is still running. The docs say entries come "during a conference transcript session" but do not promise live reads. Treat it as post-meeting.
- **Who can read:** "If you're a meeting space owner or participant, you can call the get and list methods". Scopes are `meetings.space.readonly` (sensitive), plus `drive.meet.readonly` (restricted) to download the files.
- **Availability:** the quickstart requires "A Google Workspace account with Google Meet enabled." Transcripts are available only on Business Standard and up, Enterprise, Education Plus and Workspace Individual. "Take notes for me" needs "an eligible Google Workspace edition or Google AI plan". Even with auto-transcription set up, nothing starts "until the host or co-host joins the meeting on the web". _Unverified:_ whether any of this works for a consumer @gmail organizer. Nothing documents that it does, so our test host's meetings should be assumed to produce no artifacts.
- **Setup for a self-hosted install:** the user's own Cloud project and OAuth client, as for the existing Google connector. The connections research covers the Testing-mode 7-day token caveat. A Workspace user can mark the app _Internal_.

| Feasibility                                        | Listen            | Speak | Setup                                                  | Cost                                         | Risk                                 |
| -------------------------------------------------- | ----------------- | ----- | ------------------------------------------------------ | -------------------------------------------- | ------------------------------------ |
| Good for Workspace-hosted meetings, after the fact | Yes, post-meeting | No    | Cloud project and OAuth, transcripts on in the meeting | Free API; Workspace edition with transcripts | Not live; nothing for consumer hosts |

## 3. Meet add-ons SDK and Live Sharing

The add-on client exposes meeting info, activities, frame messaging and closing the add-on. It has no audio, caption or transcript method. A human opens an add-on from the side panel. The Marketplace review checklist states: "The add-on must not provide functionality to invite a bot or rely on the presence of a bot in the meeting." A developer whose add-on was rejected for using a Recall bot asked Google for a real-time-audio alternative on 2026-07-27 and got no answer.

| Feasibility | Listen | Speak | Setup                           | Cost | Risk |
| ----------- | ------ | ----- | ------------------------------- | ---- | ---- |
| None        | No     | No    | Marketplace or a private add-on | Free | n/a  |

## 4. Accounts and bots

- **Service accounts and domain-wide delegation** let a service account "impersonate a user account" for API calls. No Meet API joins a conference as a participant, so this adds nothing beyond route 2.
- **Policy:** the Terms of Service (effective 2026-07-30) forbid "bypassing our systems or protective measures" and "hiding or misrepresenting who you are in order to violate these terms". The Meet Acceptable Use Policy forbids circumvention, including "the creation or use of multiple accounts … to engage in a behavior that was previously prohibited." `meet.google.com/robots.txt` disallows everything except the landing pages. Google sign-in "might stop sign-ins from browsers that … are being controlled through software automation rather than a human", so a human has to sign any bot account in. Cubebot's login handoff already provides that step.
- **Meeting access settings:** _Open_ lets anyone with the link join without knocking. _Trusted_ admits the host's organization, invited external users and dial-in users. _Restricted_ admits only invitees. When a host clears "Anyone can ask to join", the help center says "anonymous users or third-party bots, like note takers, that attempt to use 'Ask to join' are automatically denied". A Workspace admin can also limit meetings to signed-in users or to the organization. A personal-account host's meetings admit "All Google Account users", and those not signed in "must request to join".
- **The safeguarded admit flow** reached all account types, including personal ones, from 2026-03-24 to 2026-04-07. Risky knocks go to a second queue whose "default action … is to deny entry", and there is no admin control. Fireflies and tl;dv both document their bots appearing there as a "potential risk" or "security risk".
- **Display names:** a guest picks any name and carries the "may not be who they claim to be" note. A signed-in account shows its own name and photo, and Recall notes that "the account name overrides Create Bot's `bot_name`." Dial-in callers outside the organizer's directory appear as "Private caller".
- **A single shared bot account:** Recall's signed-in bots use a dedicated Workspace. A super admin creates a non-admin user, puts it in its own OU and points Legacy SSO at Recall. Recall estimates about 50 concurrent sessions per login and switches accounts at 30 (_unverified_, "this number isn't made public by Google"). Invited signed-in bots skip the knock in Trusted and Restricted meetings, but "a bot account from a dedicated Google Workspace is still external", and Recall adds: "Google can still apply … automated risk checks." A self-hosted Vexa user reported on 2026-11-06 that an authenticated bot was admitted within about 10 s (_unverified_, a single report).

## 5. Phone dial-in

- **Availability:** "You can only dial-in if the meeting is organized by a Google Workspace user. A dial-in number is only added when admins turn on the dial-in feature." Every Workspace edition includes a US number, and some editions add international numbers. Workspace Individual organizers also get dial-in, but personal accounts do not. Since 2026-04-16 the REST API returns a space's `phoneAccess` (number and PIN) to callers who can read it.
- **Joining:** the caller dials the number, enters the PIN and `#`, and toggles mute with `*6`. Twilio's `sendDigits` can play the PIN (`w` inserts a half-second pause). Trusted meetings admit dial-in users without a knock. In Restricted meetings they must knock, and from a different domain "someone in the meeting might need to approve you." Callers count toward the participant limit and appear as "Private caller" when they are not in the organizer's directory.
- **Two-way audio:** yes. Twilio's bidirectional Media Streams exchange 8 kHz mono μ-law over a WebSocket. Cubebot's STT and TTS would need 8 kHz resampling, and phone quality hurts STT accuracy. Callers get no captions and no speaker names: Meet sends one mixed phone stream.
- **Cost, Twilio US, August 2026:** outbound calls $0.0140/min plus Media Streams $0.0044/min, about $1.10 per hour, plus a number at $1.15/month. The call itself is an ordinary carrier call into Google's number. Other countries cost more.
- **Bot detection:** no source mentions risk screening of PSTN callers. _Unverified._

| Feasibility                                                              | Listen           | Speak | Setup                                                | Cost             | Risk                                                                            |
| ------------------------------------------------------------------------ | ---------------- | ----- | ---------------------------------------------------- | ---------------- | ------------------------------------------------------------------------------- |
| Good for Workspace-hosted meetings with dial-in; none for consumer hosts | Yes, live, mixed | Yes   | A phone provider account and number; no Google setup | About $1.10/hour | Admission in Restricted meetings; anonymous "Private caller"; low audio quality |

## 6. Meet hardware and SIP interoperability

Third-party SIP/H.323 systems reach Meet through a Pexip gateway: "The ability to turn on Meet interoperability is included with all paid Google Workspace licenses … For Pexip-based systems, you must still purchase a license from Pexip." The meeting owner's organization must turn it on, and the `gatewaySipAccess` field appears only then. That leaves out consumer-hosted meetings and any self-hosted install that does not own a licensed Pexip deployment. Google's own hardware-to-Zoom/Teams/Webex interop runs only on Meet hardware: "You can't join Teams, Zoom, SIP, or Webex meetings from the Meet web or mobile clients."

| Feasibility | Listen | Speak | Setup                                                          | Cost                     | Risk                             |
| ----------- | ------ | ----- | -------------------------------------------------------------- | ------------------------ | -------------------------------- |
| Poor        | Yes    | Yes   | Paid Workspace host organization, a Pexip license, admin setup | Pexip pricing on request | Depends on the host organization |

## 7. The user's own account

- **Companion mode** joins a second screen with "mic and video … turned off to avoid any audio feedback". Getting audio means "Leave and rejoin to use your audio". _Unverified:_ whether captions work in Companion mode. Even if they do, an agent can listen only through captions and cannot speak.
- **A second full join on the user's account** shows the user twice in the participant list, gives the agent the user's admission rights, and needs no knock in meetings the user is invited to. Google blocks automated sign-in, so the user would sign in by hand through the login handoff, and the agent's sandbox would then hold a live Google session cookie. That cookie reaches Gmail, Drive and everything else in the account, outside connection grants, approval-free tool use and the "credentials opaque to the model" rule from [Native connectors](../adr/identity.md#native-connectors). A prompt-injected agent with `bash` or the browser could use it. An agent speaking under the user's name could also run into the misrepresentation clause unless it says who it is. Not recommended.
- **The user's own browser, with the user in the call,** is how the notetakers' "bot-free" modes work. Fathom's desktop app records "the meeting locally through the Fathom Chrome extension" (macOS only) and posts a consent message in the meeting chat. Otter and Fireflies offer Chrome-extension capture as well (_vendor claims, help pages not read_). For Cubebot this would be a small extension, or a console tab with tab capture, on the person's own machine. It would forward tab audio and captions to the orchestrator, and it could inject the agent's TTS into the page's microphone the same way the meeting page script does. The agent then speaks through the user's tile, so it has to announce itself. There is no automation, no new account and no knock.

| Option                                 | Feasibility                 | Listen                          | Speak                   | Risk                                                          |
| -------------------------------------- | --------------------------- | ------------------------------- | ----------------------- | ------------------------------------------------------------- |
| Companion mode in the sandbox          | Poor                        | Captions at best (_unverified_) | No                      | User session in the sandbox                                   |
| Second join as the user in the sandbox | Works, not recommended      | Yes                             | Yes                     | Full account session exposed to the agent; identity confusion |
| Extension in the user's own browser    | Good while the user attends | Yes, live                       | Yes, as the user's tile | Only works while the user is present; must announce           |

## 8. How commercial notetakers join today

| Vendor                     | Mechanism (vendor docs)                                                            | Notes                                                                                                                                                                                                   |
| -------------------------- | ---------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Recall.ai                  | Guest bot by default; signed-in bots through a dedicated Workspace with Legacy SSO | Auto-join needs authentication. Documents `google_meet_bot_blocked` (trigger words in the name, knocking disabled). Does not mention the October refusals; "Google makes the final admission decision". |
| Fireflies                  | Calendar-invited bot, "Fireflies.ai Notetaker"; Chrome extension                   | Host must admit it; documents the "With potential risks" queue.                                                                                                                                         |
| tl;dv                      | Bot, or desktop app without a bot                                                  | Even with auto-recording "the tl;dv bot will still ask to join"; documents the red "security risk" label.                                                                                               |
| Read AI                    | Bot; botless through the Media API since 2026-03; native recording                 | Recommends native recording because Google's changes make bot joins unreliable (help article read only as a search snippet, _unverified_). Its botless path depends on the discontinued API.            |
| Fathom                     | Bot, or bot-free local capture through a Chrome extension (macOS)                  | Records only "if you are also present on the call".                                                                                                                                                     |
| Otter                      | Calendar-driven bot, "OtterPilot"; Chrome extension                                | Official help pages not read, _unverified_.                                                                                                                                                             |
| Gemini "Take notes for me" | Built into Meet, no participant                                                    | Workspace or Google AI plan; notes land in the organizer's Drive, readable through the REST API.                                                                                                        |

**No verified-bot or partner program exists.** The Media API was the only sanctioned non-human channel, and Read AI described it as "working directly with Google". None of these vendors documents an exemption from the knock or the risk queue. Vexa, an open-source bot, documents the new refusals: 403 `PERMISSION_DENIED` on `ResolveMeetingSpace` or `CreateMeetingDevice`, then "You can't join this video call". Over 24 hours to 2026-10-30, 121 hosted bots reached a verdict, 53 joined, and at least 28 were refused this way (Vexa PR #1921). Vexa responds by re-knocking up to 3 times. Cubebot should not copy that.

## 9. Google's terms on automated access

- **Terms of Service:** no abuse "by … bypassing our systems or protective measures"; no "using automated means to access content … in violation of the machine-readable instructions on our web pages (for example, robots.txt files…)"; no "hiding or misrepresenting who you are in order to violate these terms".
- **Meet Acceptable Use Policy:** no circumvention, including multiple accounts used to repeat prohibited behavior. Google "may disable your ability to use Meet or disable accounts".
- **Google APIs Terms:** applies to routes 2 and 3 only. No circumventing documented limits.
- **Reading:** a browser that joins as a clearly labeled guest and accepts the host's decision does not break an explicit Meet rule, but Meet's robots.txt disallows the meeting paths, and the risk queue and the 403 are protective measures. Retrying a refused join, changing fingerprints or rotating identities would be bypassing them. Keep the current guest join as a best-effort mode that stops at the first refusal, and say so in the console.

## What the prototype built and measured

On 2026-11-07 a full guest-join design was built on the `meetings` branch, tested against real Meet calls hosted by a consumer @gmail account, and then removed when Meet refused every join. This section keeps what was learned.

### The design

- **Assignment.** A meeting is an agent, a Meet link, a start time and a mode: _Listen_ (never speaks) or _Listen and talk_ (answers only when addressed by name, or for a follow-up within 10 s of speaking, with barge-in as in calls). Every entry point only creates an assignment; one orchestrator scheduler wakes the agent about a minute early. One agent per meeting; assigning needs `conversation.send`. Meetings were stored like calls (a `calls` row) so the transcript and summary land in the assigner's conversation.
- **Entry points.**
  - Chat: "Ayla, join <link>" assigns for now, without calling the model.
  - A Calendar page: read-only events from the Wave 1 calendar connectors, with "Assign agent" for one occurrence or a whole series.
  - An alias rule: an event whose guest list contains an agent's address is assigned automatically.
- **Joining.** A second Chromium on the agent's desktop, with a fresh profile and its own DevTools pipe. Two designs were tried:
  - Scripted clicks first. Meet ignores synthetic (untrusted) DOM clicks.
  - Then the agent's model working the Meet screens with its computer-use tool: type the name, ask to join, post "<Agent> (AI) is listening for <person>" in chat, turn on captions. This is less brittle than DOM selectors when Meet's design changes, and was the user's choice.
- **Media**, through a script injected with `Page.addScriptToEvaluateOnNewDocument`, with no PulseAudio or v4l2loopback:
  - Camera: the animated CSS cube avatar, captured with Element Capture.
  - Microphone: TTS PCM played into a `MediaStreamDestination`.
  - Hearing: remote WebRTC tracks resampled to 16 kHz.
  - Captions: read from the captions region of the DOM, with speaker names.
- **Boundary.** The page streamed to the orchestrator through the root desktop arbiter, never through the agent process. The arbiter listened on a WebSocket at `127.0.0.1:47100/meeting/<one-time token>`, accepted only Origin `https://meet.google.com`, and relayed JSON lines over the existing host channel (vsock or unix socket). The user chose this boundary; it makes the port reachable from inside the sandbox.

### Verified Chromium and Meet facts (Chromium 156, 2026-11-07)

- **Loopback access.** Meet pages reach `127.0.0.1` only after `Browser.grantPermissions` with `loopbackNetwork` (Local Network Access); without it a permission prompt hangs.
- **Trusted Types.** Meet enforces them, so injected DOM must be built without `innerHTML` (React works).
- **Cube camera.** Element Capture works without a user gesture under `--auto-accept-this-tab-capture`: `getDisplayMedia({preferCurrentTab: true, selfBrowserSurface: 'include'})`, then `track.restrictTo(RestrictionTarget.fromElement(el))` on the live CSS cube. Rendering the cube through an SVG `foreignObject` failed, with wrong 3D sorting and no animation.
- **Fake devices.**
  - `--use-fake-device-for-media-stream` breaks tab capture (`restrictTo` is missing).
  - `--use-fake-ui-for-media-stream` makes display capture pick the whole screen.
  - Overriding `MediaDevices.prototype.getUserMedia` and `enumerateDevices` works. It must be the prototype, because Meet keeps references.
- **Clicks.** Meet ignores untrusted clicks. CDP `Input.dispatchMouseEvent` clicks and `Input.insertText` are trusted and work.
- **Frames.** `addScriptToEvaluateOnNewDocument` runs in every frame, so the script must guard with `window.top === window`.
- **Joined detection.** Meet opens a connected `RTCPeerConnection` already on the pre-join screen, so peer-connection state does not mean "admitted".
- **Pre-join dialog.** On a host with real devices, Meet first shows "Do you want people to see and hear you in the meeting?"; the join flow must answer it.
- **Automation flag.** Without `--disable-blink-features=AutomationControlled`, `navigator.webdriver` is `true`, and Meet refuses on page load.
- **Docker on macOS.** A bind mount (virtiofs) can serve stale or half-written files when files are overwritten or directories are recreated; use fresh file names.

### Live join attempts

Every row is an anonymous guest on the same public IP, against meetings hosted by a consumer @gmail account. "Name screen" means Meet showed "What's your name?".

| Browser and setup                                                                                                              | Result                        |
| ------------------------------------------------------------------------------------------------------------------------------ | ----------------------------- |
| Agent-image Chromium (Linux, Xvfb, `--no-sandbox`, CDP pipe), no automation flag                                               | Refused on load               |
| Same, automation flag hidden, nothing injected                                                                                 | Name screen                   |
| …plus only the device overrides, only the `RTCPeerConnection` wrapper, only the loopback socket, or only the permission grants | Name screen                   |
| …plus `getDisplayMedia` at page load                                                                                           | Refused on load               |
| …plus `getDisplayMedia` 15 s after load                                                                                        | Name screen                   |
| Nothing injected, name typed, trusted click on _Ask to join_                                                                   | Refused right after the click |
| Same with `--use-fake-device-for-media-stream`                                                                                 | Refused right after the click |
| Same with GPU (SwiftShader WebGL) on                                                                                           | Refused on load               |
| Four knocks in one run, reloading after each refusal                                                                           | 4 of 4 refused                |
| Chrome for Testing on the Mac host, driven over CDP                                                                            | Refused right after the click |
| A human's private Firefox window                                                                                               | Joined as a guest             |

Conclusions:

- The refusal comes from Meet's check when the join request is sent ("System info will be sent to confirm you're not a bot"). It is not about the IP, Linux, the container or the injected script.
- Calling `getDisplayMedia` at load only makes the check fire earlier.
- Repeated attempts seemed to make refusals come earlier.
- No knock ever reached the host.

### Other checks

- **Plus-address invites.** Whether Google Calendar adds invites sent to a `+` address (e.g. `name+ayla@gmail.com`) to the base calendar automatically is undocumented. The only first-party answer found, a 2015 help-forum reply, says it does not. An alias rule should match whatever address an agent is given, without assuming `+`.
- **Link forms.** Meet links shared from the Meet app read `meet.google.com/meet/<code>`; a link parser must accept both forms.
- **Harness.** The tests drove Chromium over `--remote-debugging-pipe` from Python, launched through `/bin/sh -c 'exec "$0" "$@" 3<&0 4>&1'` so the pipe lands on fds 3 and 4. A small Python WebSocket server stood in for the arbiter.

## Recommendation for Cubebot

With no new Google accounts, ranked:

**Listen mode**

1. **The user's own browser forwards the meeting**, through an extension or console tab capture, while the person attends. It is live, works for consumer @gmail hosts, involves no knock and no automation, and the existing speech pipeline consumes it. It does not work if the person is absent.
2. **Meet REST API artifacts after the meeting**, when the organizer is on Workspace. Transcripts or Gemini notes go to the agent for summary and follow-up. They are not live, and there are none for consumer hosts.
3. **Guest join from the sandbox**, as the prototype did. It stays best effort: one knock, the refusal reported plainly, no retry loop.
4. **Dial-in via a phone provider**, for Workspace-hosted meetings with dial-in. It is live but delivers mixed 8 kHz audio without speaker names, and it needs a non-Google telephony account.

**Listen-and-talk mode**

1. **Dial-in via a phone provider** for Workspace-hosted meetings. It is the only documented two-way path that needs neither a Google account nor a browser. Cubebot would add a SIP/Twilio leg to the call pipeline and read `phoneAccess` from the REST API or the invite.
2. **The user's own browser**, with the agent's voice injected into the user's microphone and an announcement in chat and by voice. It works for consumer hosts but speaks under the user's identity.
3. **Guest join**, best effort, as above.
4. Not recommended: the user's Google session inside the sandbox.

**If one shared Workspace account were acceptable**, put a single non-admin user such as `agents@<their-domain>` on its own domain. A human signs it in once through the login handoff, and the agent's chat line names the agent ("Ayla (AI) via Cubebot"). That enables:

- Signed-in joins that skip the knock in Trusted and Restricted meetings when the account or its Group is invited. Calendar invites can carry it, which fits the alias rule.
- Dial-in numbers and `phoneAccess` for meetings it organizes.
- An _Internal_ OAuth app with no verification.
- Transcripts and smart notes on Business Standard. The Brazilian pricing page shows R$40.90 a month for Starter and R$81.80 for Standard.

Limits: every agent shares one Google name and photo, consumer hosts still see an external account, and signed-in joins need their own ADR, since the account's session would live in the sandbox. Whether Meet admits a signed-in automated Chromium today is only reported anecdotally (Vexa, 2026-11-06) and has to be tested before anyone relies on it. Using more than one account to get around a block would break the Acceptable Use Policy's circumvention rule.

## Open questions

- Does `transcripts.entries.list` return entries during a live meeting? This needs a test on a Workspace account.
- Do personal accounts on a Google AI plan get transcripts or REST artifacts?
- Do captions work in Companion mode?
- Is a signed-in automated join refused the same way the guest join is? This needs a test before any shared-account work.

## Sources

Google developer docs

- https://developers.google.com/workspace/meet: Media API "discontinued … New signups are now closed … endpoints shut down on January 15, 2027" (updated 2026-10-28)
- https://developers.google.com/workspace/meet/release-notes: deprecation text (2026-10-28), members GA (2026-09-11), `phoneAccess`/`gatewaySipAccess` GA (2026-04-16), smart notes GA (2026-04-02)
- https://web.archive.org/web/20260930135317/https://developers.google.com/workspace/meet/media-api/guides/overview: enrollment of all participants, consent rules, consumer meetings, encryption, minors
- https://github.com/googleworkspace/meet-media-api-samples: archived 2026-10-28; `recvonly` transceivers, "Meet only supports 3 audio virtual ssrcs", `meetings.conference.media.readonly`
- https://developers.google.com/workspace/meet/api/guides/overview: artifacts "usually ready … soon after a conference ends"; definitions of dial-in, SIP and participants
- https://developers.google.com/workspace/meet/api/guides/artifacts: owner or participant may read; 30-day transcript entries; domain-wide delegation
- https://developers.google.com/workspace/meet/api/reference/rest/v2/conferenceRecords.transcripts.entries/list
- https://developers.google.com/workspace/meet/api/guides/authenticate-authorize: scopes and sensitivity, security assessment, domain-wide delegation
- https://developers.google.com/workspace/meet/api/guides/quickstart/python: "A Google Workspace account with Google Meet enabled"
- https://developers.google.com/workspace/meet/api/guides/meeting-spaces-configuration: auto artifacts; transcripts compared with smart notes
- https://developers.google.com/workspace/meet/api/reference/rest/v2/spaces: `PhoneAccess`, `GatewaySipAccess`, access types OPEN/TRUSTED/RESTRICTED
- https://developers.google.com/workspace/events/guides/events-meet: Meet event types
- https://developers.google.com/workspace/meet/add-ons/guides/overview and https://developers.google.com/workspace/meet/add-ons/reference/websdk/addon_sdk.meetaddonclient: no media methods
- https://developers.google.com/workspace/marketplace/about-app-review: "must not provide functionality to invite a bot or rely on the presence of a bot in the meeting"
- https://discuss.google.dev/t/how-can-a-public-google-meet-add-on-access-real-time-participant-audio-without-a-bot/385365: unanswered request (2026-07-27)
- https://developers.google.com/terms: Google APIs Terms

Google help, policy and blog

- https://support.google.com/meet/answer/9303069: who can join by account type; "You can only dial-in if the meeting is organized by a Google Workspace user"; "Private caller"; US numbers in all editions
- https://support.google.com/meet/answer/9518557: phone audio; "someone in the meeting might need to approve you"
- https://support.google.com/meet/answer/12849897: editions with transcripts; start rules
- https://support.google.com/meet/answer/14754931: "Take notes for me" needs Workspace or a Google AI plan
- https://support.google.com/meet/answer/11295507: Companion mode, mic and video off
- https://support.google.com/a/users/answer/11989526: knocking off means bots "are automatically denied"
- https://workspaceupdates.googleblog.com/2026/02/safeguarded-guest-admit-flow-in-google-meet.html: deny-by-default second queue; all account types; rollout from 2026-03-24
- https://workspaceupdates.googleblog.com/2026/10/ and /2026/11/ archives: no post about refusing automated guests (checked 2026-11-07)
- https://knowledge.workspace.google.com/admin/meet-hardware/meet-interoperability-faq: Pexip licensing; not from web clients
- https://support.google.com/accounts/answer/7675428: sign-in blocked for browsers "controlled through software automation"
- https://policies.google.com/terms: Terms of Service, effective 2026-07-30
- https://support.google.com/meet/answer/9847091: Meet Acceptable Use Policy, circumvention
- https://meet.google.com/robots.txt
- https://workspace.google.com/pricing: plan prices (served in BRL from our location)

Vendors

- https://docs.recall.ai/docs/google-meet, /google-meet-faq, /google-meet-login-getting-started, /google-meet-skip-the-waiting-room, /signed-in-bots (updated 2026-04 to 2026-11)
- https://guide.fireflies.ai/articles/9554534786-how-fireflies-joins-and-records-your-meetings-faqs
- https://intercom.help/tldv/en/articles/14434461-recording-meetings-with-google-meet
- https://www.read.ai/post/read-ai-introduces-a-botless-integration-with-google-meet (2026-03-26); https://support.read.ai/hc/en-us/articles/28164245127059 (blocked; search snippet only)
- https://help.fathom.video/en/articles/449472 (edited 2026-11-05) and https://help.fathom.video/en/articles/5461313
- https://www.twilio.com/en-us/voice/pricing/us, https://www.twilio.com/docs/voice/media-streams, https://www.twilio.com/docs/voice/media-streams/websocket-messages, https://www.twilio.com/docs/voice/api/call-resource (`sendDigits`)
- https://github.com/Vexa-ai/vexa/pull/1916, https://github.com/Vexa-ai/vexa/pull/1921: refusal analysis and numbers; https://github.com/Vexa-ai/vexa/issues/2048: authenticated join admitted (2026-11-06)
- Otter: no official help page read; claims come from search results only
