# Provider subscription authentication (OpenAI and Anthropic)

Research date: October 30, 2026. This note answers the open item in [ADR 0078](../adr/0078-initial-provider-support.md): can Cubebot's native Rust engine use a person's ChatGPT/Codex or Claude subscription in place of API keys? It uses primary sources: provider documentation, terms, help-center articles and the `openai/codex` source at commit [`cbe16d5`][codex-commit] (October 30, 2026). Secondary sources are labelled. No subscription login, token exchange or inference request was run for this note. Nothing here qualifies a provider path under ADR 0078.

## Summary and recommendation

The two providers now point in opposite directions.

- **OpenAI publishes a supported path.** "Sign in with ChatGPT" (SIWC), announced at DevDay on September 29, 2026, lets an open-source or locally hosted app obtain an OAuth access token for the user's ChatGPT plan. The app then calls the public Responses API (`https://api.openai.com/v1/responses`) with that token as a bearer credential. The documentation includes a [self-hosted VM procedure][siwc-vm], and it [allows dynamic client registration with no client secret or partner key][siwc-signin]. Cubebot's OpenAI adapter already calls `/v1/responses` with `store: false` and `stream: true`, so this is an adapter change, not a new engine. [DevDay 2026][devday], [SIWC open-source overview][siwc-oss], [Models and inference][siwc-models]
- **Anthropic forbids the equivalent.** Anthropic states that third-party developers may not offer Claude.ai login in their own applications, may not route requests through Free, Pro or Max plan credentials for their users, and "may not collect, store, or intermediate Claude.ai credentials or session tokens." The only subscription use it allows on third-party infrastructure is an end user signing in to the **unmodified Claude Code binary**. [Claude Code legal and compliance][cc-legal], [Agent SDK overview][sdk-overview]

**Recommendation.**

1. Ship and qualify API-key connections for both providers first (the current ADR 0078 baseline).
2. Add an **OpenAI "ChatGPT plan" connection** through the official SIWC open-source flow. Cubebot's orchestrator would be the single SIWC host and refresh owner. Agents would receive only the one-hour access token. The connection should be limited to one subscriber's account, used for that subscriber's own agents. Mark it preview: OpenAI labels the flow a preview with request-shape limits.
3. **Do not build Anthropic subscription authentication.** Direct use is prohibited. Running Claude Code inside the sandbox conflicts with ADR 0006 (native engine) and with ADRs 0031 and 0032 (the orchestrator stores and propagates credentials). For subscription value on Anthropic, document the monthly API credits that Max, Ultra and Team plans now include. They are consumed with an ordinary API key. [API credits for subscribers][anth-credits]
4. Do not use the Codex CLI's private route (Codex's own OAuth client ID against `chatgpt.com/backend-api/codex`). OpenAI's SIWC docs tell integrators not to use `backend-api`.

## OpenAI (ChatGPT / Codex)

### Technical paths that exist

**1. Codex CLI "Sign in with ChatGPT" (the Codex app's own client).** In the Apache-2.0 [`openai/codex`][codex-repo] source:
- Login runs an OAuth authorization-code flow with PKCE and state against issuer `https://auth.openai.com`.
- The redirect goes to `http://127.0.0.1:1455/auth/callback`, falling back to port 1457.
- It requests scopes `openid profile email offline_access api.connectors.read api.connectors.invoke`.
- It stores `id_token`, `access_token`, `refresh_token` and `account_id` in an `AuthDotJson`.
- It also tries an RFC 8693 token exchange for an `openai-api-key`. Failures are ignored.

[server.rs L593–L613, L1052][codex-server]

The hard-coded refresh client ID is `app_EMoamEEZ73f0CkXaXp7hrann`, and refresh goes to `https://auth.openai.com/oauth/token`. Codex refreshes proactively when the access token is within 5 minutes of expiry, or 8 days after the last refresh. It treats `refresh_token_reused` as a permanent failure, which means refresh tokens rotate. [manager.rs L221–L234, L1774–L1872, L3528–L3545][codex-manager]

In ChatGPT mode, inference goes to `https://chatgpt.com/backend-api/codex/responses`, not the public API. [chatgpt_base_url.rs L6][codex-baseurl], [doctor.rs L3949][codex-doctor]

The user docs say credentials are cached in `~/.codex/auth.json` (plaintext) or the OS keyring, and that Codex "refreshes tokens automatically during use." For headless machines the docs recommend device-code login, or copying `auth.json` and treating it "like a password." [Codex authentication][codex-auth]

The app-server protocol also defines a `chatgptAuthTokens` mode, where a host app supplies tokens and handles refresh. It is annotated "[UNSTABLE] FOR OPENAI INTERNAL USE ONLY - DO NOT USE." [common.rs L28–L36][codex-authmode]

**2. Sign in with ChatGPT for open-source apps (the official third-party path).** As of September 29, 2026: "ChatGPT plan usage is available to open-source partners and selected private clients." [DevDay 2026][devday] The developer docs describe the flow:

- **Registration.** First sign-in uses `client_id=dynamic_agent_client` plus a stable per-host `ext_agent_host_id` and an `agent_name_hint`. The callback returns an issued `client_id` (`oaiapp_…`) "bound to the authenticated user and the workspace selected during registration." "This direct flow needs neither a client secret nor a partner API key." [Sign in][siwc-signin], [Overview][siwc-oss]
- **Endpoints and scopes.** Authorize at `https://auth.openai.com/api/accounts/authorize` and exchange at `https://auth.openai.com/api/accounts/oauth/token`, with PKCE S256 and a loopback `127.0.0.1` redirect. Scopes are `openid profile email offline_access resource.invoke chatgpt.tokens.use.direct`, with `resource=https://api.openai.com/v1`. The app must check that `chatgpt.tokens.use.direct` was granted before inference. [Sign in][siwc-signin]
- **Inference.** `POST https://api.openai.com/v1/responses` with `Authorization: Bearer <access_token>`, `store: false`, `stream: true`. The docs say: "do not point it at ChatGPT's `backend-api` endpoints." The model list comes from `GET /v1/models` with the same token. [Models and inference][siwc-models]
- **Preview limits.** System-role input items are rejected; use `instructions`. Omit `max_output_tokens`, `temperature`, `top_p`, `previous_response_id`, `metadata`, `truncation`, `user` and others. Function tools must be grouped "in namespaces or supply them through `additional_tools` input items." Hosted tools (file search, Code Interpreter, hosted MCP, `tool_search`) are unsupported. [Preview limitations][siwc-limits]
- **Self-hosted VMs.** Complete OAuth on a machine with a browser, securely transfer the credential file to the VM, keep the VM's own host ID, and "Let the VM own later refreshes." "Host-specific usage attribution and revocation of ChatGPT plan access for transferred sessions are not yet available." [Self-hosted VMs][siwc-vm]
- **Codex app-server variant.** OpenAI documents running `codex app-server` with a custom Responses provider that reads the SIWC access token from an environment variable. "Your app must refresh the OAuth token" and restart app-server with the new token. [Codex app-server][siwc-appserver]

**3. Codex access tokens (workspace automation).** "Codex access tokens are currently supported for ChatGPT Business and Enterprise workspaces." They are personal tokens that admins enable, have finite validity (for example 7–90 days) and are meant for "trusted non-interactive local workflows" through Codex CLI or app-server. They do not cover general OpenAI API calls. [Access tokens][codex-pat] They authenticate Codex, not an arbitrary Responses client, so they do not help a native engine.

### Terms of service

- **Consumer Terms of Use (effective January 1, 2026).** These prohibit "Automatically or programmatically extract data or Output" and "circumvent any rate limits or restrictions." They also say: "You may not share your account credentials or make your account available to anyone else." [Terms of Use][oai-tou] (Direct fetch returned HTTP 403. Text read from the Internet Archive capture of October 23, 2026.)
- **SIWC makes programmatic plan use an explicit, consented feature.** Users grant a separate "Use your ChatGPT plan" permission. "Using your ChatGPT plan does not … share an OpenAI API key." [Help: Using your ChatGPT plan in other apps][oai-help] (Direct fetch returned 403. Read from the Internet Archive capture of October 14, 2026; the page then read "Updated: 7 days ago.")
- **Scope conditions.** "The ChatGPT plan usage integration described here is available for open-source tools and personal projects that run locally. If you're building a paid or remotely hosted app, join the waitlist." [Cookbook: Integrating SIWC in your open-source app][oai-cookbook] The overview repeats this and links the interest form. [Overview][siwc-oss]
- **Reading for Cubebot.** Cubebot is Apache-2.0 and self-hosted. That matches "open-source and locally hosted apps," and the self-hosted VM page covers remote VMs owned by the same user. A Cubebot installation run by one party for *other people* looks like a "remotely hosted app," which needs the waitlist. That reading is an interpretation, not stated by OpenAI.
- **Codex client-ID reuse is gray-to-prohibited.** A third-party app using Codex's `app_EMoam…` client against `backend-api` would present itself as Codex. That route is outside SIWC, which explicitly excludes `backend-api`. No OpenAI statement explicitly bans it, so this is unverified, but it is the shape the consumer extraction clause targets.
- **Enforcement.** No OpenAI enforcement action against third-party subscription use was found. The direction is the opposite: partners such as OpenClaw, OpenCode, Pi and T3 are listed as open-source integrations. [Learn: Sign in with ChatGPT][learn-siwc]

### Eligibility, limits and sharing

- **Plans.** The developer docs and DevDay page say "eligible Plus and Pro" subscribers. [DevDay 2026][devday], [Learn: SIWC][learn-siwc] The help center (about October 7, 2026) says Go, Plus and Pro for commercial apps, and "Supported open-source tools remain available to all ChatGPT users." [Help][oai-help] Business, Enterprise and Edu workspace eligibility for SIWC plan usage is **not stated**; unverified.
- **Usage.** Requests count against the plan's "ChatGPT Work and Codex usage" with no new allowance. Users can cap each app's share of weekly usage and optionally allow credits after limits. [Help][oai-help] For Plus, "the five-hour usage limit is shared across all apps … including private and open-source clients"; it "does not apply for Pro users." [Accounts and sessions][siwc-sessions]
- **Errors.** Exhausted usage arrives as `subscription_sharing_usage_limit_exceeded` (HTTP 429), possibly as `response.failed` mid-stream. "OpenAI does not silently switch the request to another billing path." This matches ADR 0030's no-fallback rule. [Errors and recovery][siwc-errors]
- **Sharing.** One issued client is per user and workspace. A client can serve several hosts of the same user, each with its own host ID. [Overview][siwc-oss] Serving several humans from one subscription conflicts with the credential-sharing clause in the Terms of Use. Many agents owned by the same subscriber is not addressed. It is limited in practice by the plan's usage limits.

### Authentication lifecycle

- **Lifetimes.** Access tokens last one hour (`expires_in: 3600`). Refresh tokens last 30 days, and every refresh returns a replacement with a fresh 30-day lifetime. Rotation is mandatory: reuse fails with `refresh_token_reused`. [Token reference][siwc-tokens], [Errors and recovery][siwc-errors]
- **Refresh.** `grant_type=refresh_token` with the issued `client_id` and `resource`. "Serialize refreshes for the same session so two processes do not race a rotating token." [Accounts and sessions][siwc-sessions]
- **Revocation.** Revoke the refresh token at the OIDC `revocation_endpoint`. Users can also disconnect the app in ChatGPT settings. "OpenAI does not currently notify your tool when a user disconnects." [Accounts and sessions][siwc-sessions], [Errors and recovery][siwc-errors]

**Fit with ADR 0032.** The fit is good.
- The refresh token is a separate credential, so the orchestrator can keep it and propagate only the one-hour access token. That is the separation ADR 0032 asks for "when the provider's credential format permits."
- Mandatory rotation makes a single refresh owner a requirement. Agents refreshing independently would invalidate each other.
- Consequences: an agent cut off from the orchestrator keeps inference for at most about an hour (ADR 0012 continuation). Refresh lead time must be under an hour. A crash between a successful refresh and persistence loses the session and forces re-login. ADR 0032's "uncertain outcome" case is real here.
- Sign-in needs a browser and a loopback redirect. That means a "complete OAuth on the admin's machine and import" flow (the documented VM procedure) or a CLI helper. The web console cannot simply be the redirect target.

## Anthropic (Claude Pro / Max / Ultra / Team / Enterprise, Claude Code)

### Technical paths that exist

- **Claude Code `/login`.** This is a browser OAuth sign-in for Pro, Max, Team and Enterprise accounts (also Console). On Linux, credentials go in `~/.claude/.credentials.json` (mode 0600); on macOS, the Keychain. Claude Code refreshes the login automatically. It warns three days before expiry, and after expiry requests fail with `Login expired`. [Authentication][cc-auth]
- **`claude setup-token`.** Mints "a one-year OAuth token," used via `CLAUDE_CODE_OAUTH_TOKEN`, for "CI pipelines and scripts." It "requires a Pro, Max, Team, or Enterprise plan" and "can only make model requests." [Authentication][cc-auth]
- **Agent SDK.** The Python and TypeScript SDK "runs the Claude Code binary." For other languages, Anthropic suggests running `claude -p --output-format json` as a subprocess. Its license is the Commercial Terms. [Agent SDK overview][sdk-overview] The Claude Code binary itself is "All rights reserved," with use subject to the Commercial Terms. [claude-code LICENSE][cc-license]
- **Subscription limits.** Plan-signed-in Agent SDK and `claude -p` usage "still draw from your subscription limits." A previously announced change to Agent SDK usage was paused on June 15, 2026. [Help: Agent SDK with your Claude plan][anth-sdk-plan] (updated October 7, 2026)
- **Third-party subscription programme.** None found. Anthropic directs developers to contact sales for exceptions ("Unless previously approved"). [Agent SDK overview][sdk-overview], [Legal and compliance][cc-legal]
- **API credits bundled with plans.** Max ($100/month), Ultra ($200) and Team ($20 or $100 per seat, pooled, capped at $500) include Claude Platform credits. They are claimed by linking a Console organization and consumed with an API key. They cover "Claude API, Claude Managed Agents, the Claude Agent SDK." Free, Pro and Enterprise are not eligible. Credits expire monthly with no rollover. [API credits for subscribers][anth-credits]

### Terms of service

- **Legal and compliance page (current, accessed October 30, 2026).** It says that OAuth "is intended exclusively for purchasers of Claude Free, Pro, Max, Team, and Enterprise subscription plans and is designed to support ordinary use of Claude Code and other native Anthropic applications." It continues: "Developers building products or services … including those using the Agent SDK, should use API key authentication … Anthropic does not permit third-party developers to offer Claude.ai login into their own applications, or to route requests through Free, Pro, or Max plan credentials on behalf of their users. Moreover, developers may not collect, store, or intermediate Claude.ai credentials or session tokens — sign-in to a Claude account must complete through Anthropic's own flow." It also says: "Anthropic reserves the right to take measures to enforce these restrictions and may do so without prior notice." [Legal and compliance][cc-legal]
- **The narrow allowance.** The restriction does not "prevent an end user from signing in to the unmodified Claude Code binary with their own Claude subscription, including where a platform hosts Claude Code." A platform that preinstalls or runs Claude Code must accept the Commercial Terms, must not modify the binary or remove authentication methods, and "may not pay for, resell, or intermediate Claude usage." "Each end user must authenticate with their own" key or subscription. [Legal and compliance][cc-legal]
- **Individual use.** "Advertised usage limits for Pro and Max plans assume ordinary, individual usage of Claude Code and the Agent SDK." [Legal and compliance][cc-legal]
- **Agent SDK note.** "Unless previously approved, Anthropic does not allow third party developers to offer claude.ai login or rate limits for their products, including agents built on the Claude Agent SDK." [Agent SDK overview][sdk-overview]
- **Consumer Terms (effective October 8, 2025).** §3 prohibits accessing the Services "through automated or non-human means, whether through a bot, script, or otherwise" "Except when you are accessing our Services via an Anthropic API Key or where we otherwise explicitly permit it." §2 says: "You may not share your Account login information … with anyone else" and "may not make your Account available to anyone else." [Consumer Terms][anth-consumer] Free, Pro and Max use of Claude Code falls under these terms; Team and Enterprise fall under the Commercial Terms. [Legal and compliance][cc-legal]
- **Enforcement (secondary).** On January 9, 2026, OpenCode users reported that Claude Max authentication stopped working in OpenCode. [opencode issue #7410][opencode-7410] (secondary: third-party issue tracker; no primary Anthropic announcement found). Secondary reporting also describes a February 2026 terms update and OpenCode removing Claude OAuth after legal requests. Those details were not verified against an Anthropic primary source; the current legal page above is the authoritative statement.

### Eligibility, limits and sharing

- Pro and Max have "a five-hour session limit and a weekly limit," shared across Claude and Claude Code. Max has two tiers (5x and 20x). [Help: Claude Code with Pro or Max][anth-cc-plan] Team and Enterprise are covered by a separate article that was not read; unverified.
- Sharing one subscription across users is prohibited by Consumer Terms §2. Many unattended agents on one subscription exceeds "ordinary, individual usage."

### Authentication lifecycle

- `/login` credentials refresh automatically inside Claude Code. The access-token lifetime, refresh endpoint and rotation behaviour are not documented publicly; unverified. `setup-token` tokens last one year and are not refreshed. `/logout` revokes. [Authentication][cc-auth]
- **Fit with ADR 0032.** The fit is poor. The orchestrator storing a Claude.ai token and propagating it to sandboxes is "collect, store, or intermediate," which is prohibited regardless of how well the lifecycle would fit.

## Feasibility for Cubebot

| Option | OpenAI: ToS risk | OpenAI: effort / what breaks | Anthropic: ToS risk | Anthropic: effort / what breaks |
| --- | --- | --- | --- | --- |
| (a) Direct subscription OAuth from the Rust engine | **Low via SIWC** for an open-source, self-hosted installation used by the subscriber. High if Codex's own client and `backend-api` are reused. Waitlist needed for "paid or remotely hosted" offerings. | Medium. Add the orchestrator SIWC login, import, refresh and revoke flow. In the adapter: `instructions` instead of the system item, drop `max_output_tokens`, namespace tools or use `additional_tools`, map `subscription_sharing_*` and `{"detail":…}` errors. Breaks: hosted tools, sampling parameters, and turns longer than about 1 hour while the orchestrator is unreachable. | **Prohibited** ("may not collect, store, or intermediate"; no third-party Claude.ai login). Reported server-side blocking (secondary). | Not viable. |
| (b) Official CLI in the KVM sandbox | Low. The SIWC + `codex app-server` setup is documented. | High. It replaces the native loop, tools, memory and journal (ADR 0006, 0083). Tokens are passed as env and app-server restarts every hour. Needs Node or the Codex binary in the guest image. | Narrowly allowed only if the binary is unmodified, the end user signs in through Anthropic's own flow, the operator accepts the Commercial Terms, and usage is "ordinary, individual." Cubebot's orchestrator-held, installation-shared connection (ADR 0028, 0031, 0032) does not fit. | High, plus ADR conflicts: external engine (0006, 0078), per-sandbox human login, no orchestrator refresh, many agents per person against individual-use limits. |
| (c) Official Agent SDKs | OpenAI has no comparable subscription SDK; SIWC devkit is TypeScript. | Not Rust; would add a non-Rust runtime (AGENTS.md). | Products must use API keys. Claude.ai login needs Anthropic approval. | Not Rust; wraps the Claude Code binary, so it inherits (b). |
| (d) API keys only | None. | Already the ADR 0078 baseline. | None. Max, Ultra and Team API credits can be spent this way. | Already the baseline. Add docs on claiming plan API credits. |

**Ranked.**
- **OpenAI:** (a) via SIWC > (d) > (b) > (c). Codex-client reuse is excluded.
- **Anthropic:** (d) (with plan API credits) > (b) (only if Anthropic approves, and only with ADR changes) > (c) > (a) (prohibited).

## Open questions

1. Does a self-hosted Cubebot installation where *one* subscriber's agents serve *other* human users count as "remotely hosted"? OpenAI does not define the term; ask via the [interest form][oai-interest] before claiming multi-user support.
2. Are Business, Enterprise and Edu workspaces eligible for SIWC plan usage? Undocumented. Codex access tokens cover Business and Enterprise but only through Codex.
3. Exact tool-namespace schema required by SIWC (`namespace` grouping versus `additional_tools`): the preview-limitations page names both but the schema was not read. Needs a live test.
4. Should one Cubebot installation be one SIWC "host" (orchestrator) or should each sandbox be a host? The docs let the app decide. The orchestrator-as-host model fits ADR 0032, but OpenAI says per-host attribution and revocation for transferred sessions "are not yet available."
5. Anthropic `/login` token lifetime and rotation are undocumented. This is irrelevant unless Anthropic grants Cubebot an exception.
6. The OpenAI Terms of Use and help-center pages blocked direct fetches (HTTP 403). Their text was read from Internet Archive captures and should be rechecked in a browser.
7. Qualification under ADR 0078 still requires running the SIWC flow end to end: login, tool-using turn, refresh at under an hour, revocation, and the usage-limit error. None of that has been done.

## Sources

Accessed October 30, 2026 unless noted. Pages without a visible date are marked undated.

OpenAI

- [DevDay 2026][devday]: learn.chatgpt.com, dated September 29, 2026
- [Sign in with ChatGPT: open-source overview][siwc-oss], [Sign in][siwc-signin], [Accounts and sessions][siwc-sessions], [Models and inference][siwc-models], [Token reference][siwc-tokens], [Errors and recovery][siwc-errors], [Preview limitations][siwc-limits], [Self-hosted VMs][siwc-vm], [Codex app-server][siwc-appserver], [Request a client ID][siwc-clientid]: developers.openai.com, undated
- [Cookbook: Integrating Sign in with ChatGPT in your open-source app][oai-cookbook]: undated
- [Learn: Sign in with ChatGPT][learn-siwc], [Codex authentication][codex-auth], [Codex access tokens][codex-pat]: learn.chatgpt.com, undated
- [Help: Using your ChatGPT plan in other apps and sites][oai-help]: Internet Archive capture of October 14, 2026 ("Updated: 7 days ago")
- [Terms of Use][oai-tou]: effective January 1, 2026; Internet Archive capture of October 23, 2026
- [openai/codex][codex-repo] at [`cbe16d5`][codex-commit] (October 30, 2026): [login/src/server.rs][codex-server], [login/src/auth/manager.rs][codex-manager], [model-provider/src/chatgpt_base_url.rs][codex-baseurl], [cli/src/doctor.rs][codex-doctor], [app-server-protocol/src/protocol/common.rs][codex-authmode]

Anthropic

- [Claude Code legal and compliance][cc-legal]: undated, current
- [Claude Code authentication][cc-auth]: undated, references v2.1.299
- [Agent SDK overview][sdk-overview]: undated
- [Consumer Terms of Service][anth-consumer]: effective October 8, 2025
- [API credits for Max, Ultra, and Team plans][anth-credits]: undated
- [Help: Use the Claude Agent SDK with your Claude plan][anth-sdk-plan]: updates of June 15 and October 7, 2026
- [Help: Use Claude Code with your Pro or Max plan][anth-cc-plan]: "Updated this week"
- [anthropics/claude-code LICENSE.md][cc-license]

Secondary

- [anomalyco/opencode issue #7410][opencode-7410]: January 9, 2026. User report of the Claude Max block; no primary Anthropic announcement located.

[devday]: https://learn.chatgpt.com/docs/whats-new/devday-2026
[siwc-oss]: https://developers.openai.com/siwc/token-sharing-open-source
[siwc-signin]: https://developers.openai.com/siwc/token-sharing-open-source/sign-in
[siwc-sessions]: https://developers.openai.com/siwc/token-sharing-open-source/profiles-and-sessions
[siwc-models]: https://developers.openai.com/siwc/token-sharing-open-source/models-and-inference
[siwc-tokens]: https://developers.openai.com/siwc/token-sharing-open-source/token-reference
[siwc-errors]: https://developers.openai.com/siwc/token-sharing-open-source/errors-and-recovery
[siwc-limits]: https://developers.openai.com/siwc/token-sharing-open-source/preview-limitations
[siwc-vm]: https://developers.openai.com/siwc/token-sharing-open-source/self-hosted-vms
[siwc-appserver]: https://developers.openai.com/siwc/token-sharing-open-source/codex-app-server
[siwc-clientid]: https://developers.openai.com/siwc/request-client-id
[oai-cookbook]: https://developers.openai.com/cookbook/articles/sign-in-with-chatgpt
[oai-interest]: https://openai.com/form/sign-in-with-chatgpt-interest/
[learn-siwc]: https://learn.chatgpt.com/docs/sign-in-with-chatgpt
[codex-auth]: https://learn.chatgpt.com/docs/auth
[codex-pat]: https://learn.chatgpt.com/docs/enterprise/access-tokens
[oai-help]: https://web.archive.org/web/20261014092807/https://help.openai.com/en/articles/20001542-using-your-chatgpt-plan-in-other-apps-and-sites
[oai-tou]: https://web.archive.org/web/20261023115502/https://openai.com/policies/row-terms-of-use/
[codex-repo]: https://github.com/openai/codex
[codex-commit]: https://github.com/openai/codex/commit/cbe16d525c2d7362fc3d4726143288e932905daa
[codex-server]: https://github.com/openai/codex/blob/cbe16d525c2d7362fc3d4726143288e932905daa/codex-rs/login/src/server.rs#L593-L613
[codex-manager]: https://github.com/openai/codex/blob/cbe16d525c2d7362fc3d4726143288e932905daa/codex-rs/login/src/auth/manager.rs#L221-L234
[codex-baseurl]: https://github.com/openai/codex/blob/cbe16d525c2d7362fc3d4726143288e932905daa/codex-rs/model-provider/src/chatgpt_base_url.rs#L6
[codex-doctor]: https://github.com/openai/codex/blob/cbe16d525c2d7362fc3d4726143288e932905daa/codex-rs/cli/src/doctor.rs#L3949
[codex-authmode]: https://github.com/openai/codex/blob/cbe16d525c2d7362fc3d4726143288e932905daa/codex-rs/app-server-protocol/src/protocol/common.rs#L28-L36
[cc-legal]: https://code.claude.com/docs/en/legal-and-compliance
[cc-auth]: https://code.claude.com/docs/en/authentication
[sdk-overview]: https://code.claude.com/docs/en/agent-sdk/overview
[anth-consumer]: https://www.anthropic.com/legal/consumer-terms
[anth-credits]: https://platform.claude.com/docs/en/about-claude/api-credits-for-subscribers
[anth-sdk-plan]: https://support.claude.com/en/articles/15036540-use-the-claude-agent-sdk-with-your-claude-plan
[anth-cc-plan]: https://support.claude.com/en/articles/11145838-use-claude-code-with-your-pro-or-max-plan
[cc-license]: https://github.com/anthropics/claude-code/blob/main/LICENSE.md
[opencode-7410]: https://github.com/anomalyco/opencode/issues/7410
