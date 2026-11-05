# Subscription login as a model provider

Research date: 2026-11-05. Question: can Cubebot let a user "log in with their AI subscription" (Claude Pro/Max, ChatGPT Plus/Pro, Google AI Pro/Ultra or free Gemini, SuperGrok/X Premium) and use that login instead of a paid API key?

Method: vendor docs, terms and FAQ pages, vendor GitHub repos, and competitor source code (shallow clones on 2026-11-05: `openai/codex` b2a8e68, `google-gemini/gemini-cli` eb5a1ba, `anomalyco/opencode` ae0fde7, `openclaw/openclaw` cf80b26, `xai-org/grok-build` HEAD). Each claim below is tagged:

- **[V]** verified from a vendor's own page or source code (link given).
- **[C]** community or secondary claim, not traced to the vendor. Treat as unconfirmed.

Nothing here is legal advice. Terms change often; re-check the linked pages before shipping.

## TL;DR

| Vendor    | Path                                                                                                                                           | Status                                                                                     | Risk to the user's account                                                                |
| --------- | ---------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------ | ----------------------------------------------------------------------------------------- |
| OpenAI    | **Sign in with ChatGPT, "ChatGPT plan usage" for open-source apps** (own OAuth client via dynamic registration, `api.openai.com/v1/responses`) | **(a) Officially supported** (preview)                                                     | Low. This is the program built for this. Hosted or paid apps need OpenAI's approval first |
| OpenAI    | Drive `codex exec` / `codex app-server` with the user's own Codex login                                                                        | (a) Supported (app-server is documented for integrations)                                  | Low                                                                                       |
| OpenAI    | Reuse Codex's client id `app_EMoam…` against `chatgpt.com/backend-api/codex`                                                                   | (b) Works, unsanctioned. OpenCode now labels it "legacy"                                   | Medium. OpenAI says not to use `backend-api`                                              |
| Anthropic | Raw OAuth / `setup-token` used directly against `api.anthropic.com` (spoofed Claude Code headers)                                              | **(b) Prohibited in writing; enforced since Jan 2026**                                     | High. Server-side blocks; Anthropic "may [enforce] without prior notice"                  |
| Anthropic | Drive the **unmodified** `claude` binary (`claude -p` / Agent SDK), user signs in through Anthropic's own flow                                 | **Narrowly allowed, with conditions** (see §1.2)                                           | Medium. "Ordinary, individual usage" limits; Cubebot must never handle the token          |
| Anthropic | Cubebot offers a "Log in with Claude" button                                                                                                   | (c) Not permitted ("Unless previously approved")                                           | n/a                                                                                       |
| Google    | Gemini CLI "Login with Google" OAuth (`cloudcode-pa.googleapis.com`)                                                                           | **(c) Gone**: consumer Login with Google ended 2026-06-18; third-party use was also banned | High. Suspensions reported Feb 2026                                                       |
| Google    | Antigravity OAuth, or wrapping `agy -p` headless                                                                                               | **(b) Violates terms / "not supported under consumer plans"**                              | High. Terms §6 names this case                                                            |
| Google    | Gemini API key (AI Studio) or Vertex                                                                                                           | (a) The vendor's stated route for third-party agents                                       | None (Cubebot already does this)                                                          |
| xAI       | SuperGrok / X Premium OAuth (device code, `auth.x.ai`, `cli-chat-proxy.grok.com/v1`)                                                           | **Endorsed for named partners (OpenCode, Kilo Code)**; no public program for other apps    | Low to medium. Reuses Grok CLI's client id; no written permission for Cubebot             |
| xAI       | Drive `grok -p` / `grok agent` (ACP) with the user's login                                                                                     | Documented ("build integrations")                                                          | Low                                                                                       |

**Recommendation in one line:** ship OpenAI first through the official Sign in with ChatGPT program. Offer xAI SuperGrok as an opt-in labelled "partner-endorsed, not yet approved for Cubebot", or ask xAI first. Offer Claude only as an optional "Claude Code runtime" that drives the user's own unmodified `claude` binary, never through a Cubebot-managed token. Keep Google on API keys.

---

## 1. Anthropic (Claude Pro/Max/Ultra)

### 1.1 Policy (verified)

Claude Code _Legal and compliance_, section "Authentication and credential use" ([code.claude.com/docs/en/legal-and-compliance](https://code.claude.com/docs/en/legal-and-compliance)) [V]:

> **OAuth authentication** is intended exclusively for purchasers of Claude Free, Pro, Max, Ultra, Team, and Enterprise subscription plans and is designed to support ordinary use of Claude Code and other native Anthropic applications.

> **Developers** building products or services that interact with Claude's capabilities, including those using the Agent SDK, should use API key authentication through Claude Console or a supported cloud provider. Anthropic does not permit third-party developers to offer Claude.ai login into their own applications, or to route requests through Free, Pro, Max, or Ultra plan credentials on behalf of their users. Moreover, developers may not collect, store, or intermediate Claude.ai credentials or session tokens — sign-in to a Claude account must complete through Anthropic's own flow.

> … Nor does it prevent an end user from signing in to the unmodified Claude Code binary with their own Claude subscription, including where a platform hosts Claude Code as described under _Can customers offer Claude Code in their products?_ above.

> Anthropic reserves the right to take measures to enforce these restrictions and may do so without prior notice.

The same page, "Can customers offer Claude Code in their products?" [V]:

> Unless we've mutually agreed otherwise, preinstalling or running Claude Code in your products or services (e.g. in hosted sandboxes or other agent infrastructure) requires agreeing to our Commercial Terms of Service and complying with the conditions below:
>
> - **The Claude Code binary must not be modified.** … customers may not remove, disable, or restrict any authentication method built into it …
> - **Customers may not pay for, resell, or intermediate Claude usage on their end users' behalf.** Each end user must authenticate with their own Anthropic API key, Claude subscription plan credentials, or 3P inference provider credential …

And, under "Acceptable use" [V]: "Advertised usage limits for Pro, Max, and Ultra plans assume ordinary, individual usage of Claude Code and the Agent SDK."

Agent SDK overview ([code.claude.com/docs/en/agent-sdk/overview](https://code.claude.com/docs/en/agent-sdk/overview)) [V]:

> Unless previously approved, Anthropic does not allow third party developers to offer claude.ai login or rate limits for their products, including agents built on the Claude Agent SDK.

Branding [V], same page: you may not call your product "Claude Code" or mimic it.

Consumer Terms (effective Oct 8, 2025 per page metadata; [anthropic.com/legal/consumer-terms](https://www.anthropic.com/legal/consumer-terms)) [V]: §3 forbids, "Except when you are accessing our Services via an Anthropic API Key or where we otherwise explicitly permit it, to access the Services through automated or non-human means, whether through a bot, script, or otherwise". §2: "You may not share your Account login information … or Account credentials with anyone else. You also may not make your Account available to anyone else."

Billing status [V] ([support.claude.com article 15036540](https://support.claude.com/en/articles/15036540-use-the-claude-agent-sdk-with-your-claude-plan)): June 15, 2026 update, "We've paused the previously-announced changes to Claude Agent SDK usage"; Agent SDK, `claude -p`, "and third-party app usage still draw from your subscription limits". Oct 7, 2026: Max, Ultra and Team plans now include monthly API credits, usable with a linked Console API key. The paused change (May 2026) would have moved SDK, `claude -p` and third-party usage onto a separate monthly credit [C] ([The New Stack](https://thenewstack.io/anthropic-pauses-claude-agent-sdk-subscription-change/)).

### 1.2 What this means for each path

1. **Cubebot does its own OAuth, or stores a `setup-token`, and calls `api.anthropic.com` itself.** Prohibited: this is "offer Claude.ai login", "route requests through … plan credentials", and "collect, store, or intermediate … session tokens". Third-party tools do this today by spoofing Claude Code. OpenClaw's source sends `anthropic-beta: claude-code-20250219,oauth-2025-04-20` and `user-agent: claude-cli/<version>` on setup-token requests ([openclaw `extensions/anthropic/stream-wrappers.ts` L42–69](https://github.com/openclaw/openclaw/blob/main/extensions/anthropic/stream-wrappers.ts)) [V, as competitor code]. Its docs say requests "advertise a maintained Claude Code client version … Anthropic uses that identity to gate newer models" ([openclaw docs/providers/anthropic.md](https://github.com/openclaw/openclaw/blob/main/docs/providers/anthropic.md)). Anthropic does not document the OAuth endpoints or client id for third parties. Community reports put the client id at `9d1c250a-…` [C]; I did not verify it, because the path is off-limits anyway.
2. **Cubebot runs the unmodified `claude` binary (`claude -p --output-format stream-json`, or the Agent SDK) inside the agent sandbox, and the user signs in with `claude auth login` through Anthropic's own browser flow.** This is the only path the text leaves open ("Nor does it prevent an end user from signing in to the unmodified Claude Code binary with their own Claude subscription, including where a platform hosts Claude Code"). Conditions and grey areas:
   - The binary must stay unmodified, with all its login methods intact.
   - Cubebot must not read, copy, vault or propagate the resulting credential. That rules out the current [orchestrator-owned refresh](../adr/identity.md#orchestrator-owned-provider-authentication-refresh) and [propagated authentication](../adr/agents.md#direct-provider-inference-with-propagated-authentication) model for this provider. The binary owns its own `~/.claude/.credentials.json` and its refresh.
   - `claude setup-token` + `CLAUDE_CODE_OAUTH_TOKEN` is documented "for CI pipelines, scripts" ([authentication docs](https://code.claude.com/docs/en/authentication)) [V]. Pasting that token into Cubebot's vault would mean Cubebot "stores" a session token. **Do not** build a "paste your setup-token" field.
   - The hosting clause says platforms that preinstall or run Claude Code "require agreeing to our Commercial Terms". For a self-hosted Cubebot the operator _is_ the subscriber. Whether the Cubebot project, or each self-hosting user, counts as "customer" here is not stated. **Open question.**
   - "Ordinary, individual usage": always-on agents, dreaming and multiple parallel agents on one Pro/Max login are plausibly outside "ordinary". The account at risk is the user's own.
   - The SDK note "Unless previously approved, Anthropic does not allow third party developers to offer claude.ai login … including agents built on the Claude Agent SDK" means Cubebot must not present it as "Log in with Claude". The UI should say "use your installed Claude Code" and send the user to the binary's own `/login`.
3. **Officially sanctioned alternatives:** an API key, or a Max/Ultra/Team plan's monthly API credits through a linked Console org API key (Oct 7, 2026 update above) [V]. This is the cleanest "use what you already pay for" story for Anthropic, and Cubebot already supports API keys.

### 1.3 Enforcement timeline

- 2026-01-09: server-side blocking of subscription OAuth tokens outside Claude Code; OpenCode, Cline, Roo and OpenClaw broke [C] ([Gigazine](https://gigazine.net/gsc_news/en/20260220-anthropic-third-party-block), [falcao.org](https://falcao.org/posts/anthropic-claude-access-crackdown-ecosystem-fallout/)).
- 2026-02-19: legal-and-compliance page updated with the text above [C for the date; V for the current text].
- 2026-03: OpenCode removed Anthropic OAuth after a legal demand [C]. OpenCode's own docs now say of Claude Pro/Max plugins: "Anthropic explicitly prohibits this" and bundled plugins were removed "as of 1.3.0" ([opencode.ai/docs/providers](https://opencode.ai/docs/providers/)) [V for OpenCode's statement].
- 2026-04-04: blocking extended to all third-party harnesses, OpenClaw first [C] ([alternativeto](https://alternativeto.net/news/2026/4/anthropic-ends-free-claude-access-for-third-party-tools-like-openclaw)).

### 1.4 Technical notes for the binary-driving path

- Headless: `claude -p` with `--output-format json` / `stream-json`. Anthropic itself recommends this for non-Python and non-TypeScript callers: "To drive the same agent loop from a language other than Python or TypeScript, run the CLI as a subprocess with the `-p` flag" ([Agent SDK overview](https://code.claude.com/docs/en/agent-sdk/overview)) [V].
- Credentials: Linux `~/.claude/.credentials.json` mode 0600; `CLAUDE_CONFIG_DIR` separates accounts; logins expire, with a warning 3 days before; "Once the stored login expires and can't be refreshed, each model request fails with `Login expired`". `--bare` mode does not read `CLAUDE_CODE_OAUTH_TOKEN` ([authentication](https://code.claude.com/docs/en/authentication)) [V].
- Precedence: `ANTHROPIC_API_KEY` beats the subscription login, and `-p` always uses the key if present. Cubebot must not inject an API key into that sandbox's environment [V, same page].
- Consequence: Claude Code becomes the agent loop with its own tools, not Cubebot's Rust engine. That conflicts with "The agent engine owns its local model and tool loop" ([agents ADR](../adr/agents.md#direct-provider-inference-with-propagated-authentication)) and needs its own ADR.

---

## 2. OpenAI (ChatGPT Plus/Pro, Codex)

### 2.1 Policy (verified): there is an official program

OpenAI runs **Sign in with ChatGPT** with an optional **"ChatGPT plan usage"** capability for third-party apps ([developers.openai.com/siwc](https://developers.openai.com/siwc)) [V]. It is described as letting users "sign in with ChatGPT and use their ChatGPT plan for eligible AI requests in your app."

Eligibility ([siwc/token-sharing-open-source](https://developers.openai.com/siwc/token-sharing-open-source)) [V]:

> These docs explain ChatGPT plan usage for open-source and locally hosted apps. … If you're interested in offering it in a paid or remotely hosted app, complete the interest form ([openai.com/form/sign-in-with-chatgpt-interest](https://openai.com/form/sign-in-with-chatgpt-interest/)).

There is a dedicated **Self-hosted VMs** guide: "Use this guide when you run an open-source app on a remote virtual machine (VM)" ([siwc/…/self-hosted-vms](https://developers.openai.com/siwc/token-sharing-open-source/self-hosted-vms)) [V]. Cubebot (Apache-2.0, self-hosted) fits "open-source app". A self-hosted server is arguably not "remotely hosted" in the SaaS sense, but confirm with OpenAI (see open questions).

The user-facing side ([learn.chatgpt.com/docs/sign-in-with-chatgpt](https://learn.chatgpt.com/docs/sign-in-with-chatgpt)) [V]: "In supported apps, eligible ChatGPT Plus and Pro subscribers can also choose to use their ChatGPT plan for AI requests." "App usage counts toward your existing plan limits." Users can cap each app as a percentage of weekly plan usage. "Using your plan does not give the app access to your ChatGPT conversations or memories." The help-center article ([help.openai.com 20001542](https://help.openai.com/en/articles/20001542-using-your-chatgpt-plan-in-other-apps-and-sites)) returned HTTP 403 to my fetcher. A search snippet says it lists OpenClaw, OpenCode, Pi and T3 as open-source integrations [C].

### 2.2 Mechanism: official SIWC open-source flow (verified)

From [sign-in](https://developers.openai.com/siwc/token-sharing-open-source/sign-in), [models-and-inference](https://developers.openai.com/siwc/token-sharing-open-source/models-and-inference), [token-reference](https://developers.openai.com/siwc/token-sharing-open-source/token-reference), [preview-limitations](https://developers.openai.com/siwc/token-sharing-open-source/preview-limitations), [profiles-and-sessions](https://developers.openai.com/siwc/token-sharing-open-source/profiles-and-sessions) [V]:

- **Authorize:** `https://auth.openai.com/api/accounts/authorize` in the system browser. `response_type=code`, PKCE `S256`, fresh `state` and `nonce`, `resource=https://api.openai.com/v1`.
- **Client registration:** the first sign-in uses `client_id=dynamic_agent_client` plus `agent_name_hint=<app name>`. The callback returns an issued client id (`oaiapp_…`); save and reuse it. "No client secret or partner API key is required."
- **Host id:** `ext_agent_host_id` is required, stable, opaque, one per host (`urn:ietf:params:oauth:jwk-thumbprint:…` recommended, or `urn:uuid:…`).
- **Scopes:** `openid profile email` for identity, plus `offline_access resource.invoke chatgpt.tokens.use.direct` for plan usage.
- **Redirect:** loopback `http://127.0.0.1:<port>/auth/callback`. "Do not substitute with `localhost`." Only the port may change later. **No device-code flow is documented** for this program.
- **Token:** `https://auth.openai.com/api/accounts/oauth/token`, form-encoded `authorization_code` with `code_verifier`, the same `redirect_uri` and the same `resource`.
- **Lifetimes:** access token 1 h (`expires_in: 3600`); refresh token 30 days, **rotating** ("Each successful refresh returns a replacement refresh token with a fresh 30-day lifetime"). "Serialize refreshes for the same session so two processes do not race a rotating token."
- **Inference:** `POST https://api.openai.com/v1/responses` with `Authorization: Bearer <access_token>`. Models come from `GET https://api.openai.com/v1/models`. Filter `visibility == "list"` and use `slug`. "Do not point it at ChatGPT's `backend-api` endpoints."
- **Request constraints:** `store: false` and `stream: true` are required. Explicit `{type:"message", role:"system"}` items are rejected; use `instructions` or developer messages. Omit `background, conversation, max_output_tokens, max_tool_calls, metadata, moderation, multi_agent, prompt, prompt_cache_retention, safety_identifier, temperature, top_logprobs, top_p, truncation, user`, and `previous_response_id` over HTTP. Function tools must be grouped in namespaces or passed via `additional_tools`. Hosted tools such as image generation, file search, code interpreter, computer use and hosted MCP are unsupported. Audio/video input is unsupported.
- **Usage-limit errors** can arrive mid-stream as `response.failed` with `subscription_sharing_usage_limit_exceeded` or `subscription_sharing_usage_unavailable`.
- **Remote/VM deployments:** complete OAuth on the machine with the browser, copy the credential file (client_id, access_token, refresh_token, id_token) to the VM over a secure channel, and keep the VM's own host id. "Let the VM own later refreshes." Caveat: "Host-specific usage attribution and revocation of ChatGPT plan access for transferred sessions are not yet available."
- **Accounts:** "Keep each registration's … issued client ID, and credentials separate … Never overwrite another account registration's credentials."
- It is a **preview**; the limitations page is titled that way.

Reference implementation: OpenCode's `packages/core/src/plugin/provider/chatgpt.ts` implements exactly this ("Sign in with ChatGPT", `dynamic_agent_client`, `ext_agent_host_id`, `resource=https://api.openai.com/v1`) [V, competitor source].

### 2.3 Legacy / alternative OpenAI paths

- **Codex CLI client id against `chatgpt.com/backend-api/codex`.** Codex's own client id is `app_EMoamEEZ73f0CkXaXp7hrann` (`codex-rs/tui/src/onboarding/auth.rs` L1248) and it calls `https://chatgpt.com/backend-api/codex/responses` (`codex-rs/cli/src/doctor.rs` L4022) [V, openai/codex]. OpenCode still ships this as "Codex browser (legacy)" and "Codex device code (legacy)" (`packages/core/src/plugin/provider/openai.ts`), with device auth at `auth.openai.com/api/accounts/deviceauth/usercode` [V, competitor source]. This now contradicts OpenAI's "do not point it at ChatGPT's backend-api" guidance. Avoid it.
- **Drive the Codex binary.** `codex exec`, or `codex app-server --listen stdio://` (JSON-RPC `initialize` → `thread/start` → `turn/start`). OpenAI documents app-server under SIWC: the app passes its own SIWC access token through `env_key` with `model_provider` `wire_api="responses"`, and on renewal restarts app-server and `thread/resume`s ([siwc/…/codex-app-server](https://developers.openai.com/siwc/token-sharing-open-source/codex-app-server)) [V]. The Codex auth page also documents `codex login --device-auth` (beta; must be enabled in ChatGPT security settings), `~/.codex/auth.json`, auto-refresh, and that "API keys are still the recommended default for automation" ([learn.chatgpt.com/docs/auth](https://learn.chatgpt.com/docs/auth)) [V].
- Plan note [V]: "As of August 31, 2026, GPT-5.4 and GPT-5.4 mini are no longer available in Codex when you sign in with your ChatGPT account" (search snippet of [learn.chatgpt.com changelog](https://learn.chatgpt.com/docs/changelog); I did not open the page itself). The model list should come from `/v1/models` at runtime, not be hard-coded.

### 2.4 Enforcement

No OpenAI enforcement against third-party ChatGPT login found. OpenAI instead built the program above. OpenCode announced ChatGPT Plus/Pro support in v1.1.11 on 2026-01-09/10 ([x.com/thdxr](https://x.com/thdxr/status/2009803906461905202)) [C, vendor of OpenCode, not OpenAI].

---

## 3. Google (Gemini CLI / Code Assist / Antigravity)

### 3.1 Policy (verified)

- **Gemini CLI terms** ([gemini-cli docs/resources/tos-privacy.md](https://github.com/google-gemini/gemini-cli/blob/main/docs/resources/tos-privacy.md), added in commit [83a3851d, 2026-02-27](https://github.com/google-gemini/gemini-cli/commit/83a3851d)) [V]:
  > Directly accessing the services powering Gemini CLI (for example, the Gemini Code Assist service) using third-party software, tools, or services (for example, using OpenClaw with Gemini CLI OAuth) is a violation of applicable terms and policies. Such actions may be grounds for suspension or termination of your account.
- **Gemini CLI FAQ** ([docs/resources/faq.md](https://github.com/google-gemini/gemini-cli/blob/main/docs/resources/faq.md)) [V]: "Using third-party software, tools, or services to harvest or piggyback on Gemini CLI's OAuth authentication to access our backend services is a direct violation … If you would like to use a third-party coding agent with Gemini, the supported and secure method is to use a Vertex AI or Google AI Studio API key."
- **Consumer Login with Google ended.** [developers.google.com/gemini-code-assist/docs/deprecations/code-assist-individuals](https://developers.google.com/gemini-code-assist/docs/deprecations/code-assist-individuals) [V]: effective **June 18, 2026**, for "the Gemini Code Assist for individuals, Google AI Pro, and Google AI Ultra tiers", "you can no longer use the Login with Google option to access the IDE extensions or Gemini CLI". Consumers are pointed to Antigravity. (The Gemini CLI repo docs still describe the old login flow and quotas of 1,000/1,500/2,000 requests per day; that is stale relative to this deprecation.)
- **Antigravity terms §6** ([antigravity.google/terms](https://antigravity.google/terms); no date on page) [V]:
  > You must not abuse, harm, interfere with, or disrupt the Service. This includes, but is not limited to, using the Service in connection with products not provided by us. Using third party software, tools, or services to access the Service (e.g. using OpenClaw with Antigravity OAuth) is a breach of this Agreement. Such actions may be grounds for suspension or termination of your Antigravity and/or Gemini CLI accounts.
- **Antigravity FAQ** ([antigravity.google/docs/faq](https://antigravity.google/docs/faq)) [V]:
  > Antigravity CLI headless mode (`agy -p`) is intended for direct scripting and CI workflows. Wrapping agy inside third-party coding agents or applications, or automating account rotation, is not supported under consumer plans.
  > … To use a third-party coding agent with Gemini, we recommend using a Gemini Enterprise or Google AI Studio API key.

Community forum answers claiming wrapping `agy -p` is "fully supported" come from users with no Google staff role shown, and they contradict the FAQ above ([thread 183051](https://discuss.ai.google.dev/t/is-external-orchestration-of-antigravity-cli-headless-mode-supported-with-account-based-usage/183051), [thread 183429](https://discuss.ai.google.dev/t/is-it-allowed-for-an-open-source-tool-to-call-the-official-agy-cli-in-headless-mode/183429)) [C]. The FAQ governs.

### 3.2 Mechanism (historical, for completeness)

From `gemini-cli/packages/core/src/code_assist/` [V]: installed-app Google OAuth client `681255809395-oo8ft2oprdrnp9e3aqf6av3hmdib135j.apps.googleusercontent.com` (its secret is embedded in the source, as Google allows for installed apps); scopes `cloud-platform`, `userinfo.email` and `userinfo.profile`; loopback `http://127.0.0.1:<port>/oauth2callback`, or `NO_BROWSER=true` with the redirect `https://codeassist.google.com/authcode`; backend `https://cloudcode-pa.googleapis.com/v1internal:{loadCodeAssist,onboardUser,generateContent,streamGenerateContent,retrieveUserQuota}`. That is a private Code Assist API with a Gemini-shaped but wrapped request body, not the public Gemini API. Driving `gemini --acp` is documented (`docs/cli/acp-mode.md`), but for consumer accounts the login itself no longer exists.

### 3.3 Enforcement

- Feb 2026: Google AI Pro/Ultra accounts were restricted for using Antigravity/Gemini CLI OAuth through OpenClaw ([openclaw issue #14203](https://github.com/openclaw/openclaw/issues/14203); [Google AI forum thread](https://discuss.ai.google.dev/t/account-restricted-without-warning-google-ai-ultra-oauth-via-openclaw/122778)). The forum quotes a support reply calling it a ToS violation under "zero tolerance", "not reversible" [C: user-posted copy of a Google reply]. A blog claims Google later reversed some bans ([metricnexus](https://metricnexus.ai/blog/google-banning-openclaw-antigravity-users)) [C, unverified].
- OpenClaw now says it "does not offer new Gemini CLI OAuth or Antigravity OAuth setup" and cites the deprecation and Antigravity terms ([openclaw docs/providers/google.md](https://github.com/openclaw/openclaw/blob/main/docs/providers/google.md)) [V, competitor doc].

---

## 4. xAI (SuperGrok / X Premium)

### 4.1 Policy

- xAI publicly endorses subscription login in two named third-party tools [V, fetched with curl because WebFetch was blocked]:
  - "Use Grok in OpenCode" (May 21, 2026, [x.ai/news/grok-opencode](https://x.ai/news/grok-opencode)): "Use your SuperGrok or X Premium subscription inside OpenCode." It offers browser OAuth and "Headless / Remote / VPS" device code. "More open-source agents and integrations are coming soon."
  - "Use Grok in Kilo Code" (May 27, 2026, [x.ai/news/grok-kilocode](https://x.ai/news/grok-kilocode)): "Use your SuperGrok or X Premium+ subscription inside Kilo Code … with no separate API key."
- I found **no public xAI developer program**, client-registration flow or terms text that permits _other_ apps to use subscription OAuth, and none that forbids it. The consumer ToS ([x.ai/legal/terms-of-service](https://x.ai/legal/terms-of-service)) was Cloudflare-blocked to both WebFetch and curl (HTTP 403), so **I could not read its automation clauses**. Not verified.
- Plan pricing page (search snippet, [x.ai/pricing](https://x.ai/pricing)) [C]: paid plans "include Grok API access", but API usage is "billed separately at standard model rates and doesn't draw from your plan's usage pool". So the plan does not turn into free `api.x.ai` usage. Subscription usage goes through the CLI proxy below.

### 4.2 Mechanism (verified)

- OIDC discovery ([auth.x.ai/.well-known/openid-configuration](https://auth.x.ai/.well-known/openid-configuration)) [V]: authorize `https://auth.x.ai/oauth2/authorize`, token `/oauth2/token`, **device authorization `/oauth2/device/code`**, revoke `/oauth2/revoke`; grants `authorization_code`, `refresh_token`, `urn:ietf:params:oauth:grant-type:device_code`; PKCE S256; token-endpoint auth `none` is allowed (public clients).
- Client id `b1a00492-073a-47ea-816f-4c329264a828` is **Grok CLI's own** (`xai-org/grok-build` `crates/codegen/xai-grok-pager/scripts/install.sh` L154) [V]. OpenCode (`packages/core/src/plugin/provider/xai.ts`) and OpenClaw (`extensions/xai/xai-oauth.ts` L34) reuse it, with scope `openid profile email offline_access grok-cli:access api:access`; OpenCode adds `referrer: "opencode"` [V, competitor source]. That is a borrowed first-party client id, tolerated in practice for the endorsed partners.
- Inference: `https://cli-chat-proxy.grok.com/v1/responses` (Responses-shaped), confirmed by grok-build's error fixture (`xai-grok-pager/src/app/error_display.rs` L774) and default base URL (`xai-grok-workspace/src/handle.rs` L4560) [V]. OpenClaw uses the same base URL with the `openai-responses` API and a billing endpoint at `/v1/billing?format=credits` [V, competitor source].
- Grok CLI auth docs ([grok-build 02-authentication.md](https://github.com/xai-org/grok-build/blob/main/crates/codegen/xai-grok-pager/docs/user-guide/02-authentication.md)) [V]: `grok login --device-auth` for headless; `~/.grok/auth.json` 0600; background refresh 300 s before expiry; 30-day fallback lifetime; `XAI_API_KEY` as fallback.
- Driving the binary: `grok -p` headless mode "to automate tasks, script workflows, build integrations" (`14-headless-mode.md`), and `grok agent` over ACP for "IDEs, SDKs, eval harnesses, and custom apps" (`15-agent-mode.md`) [V]. This is the most clearly sanctioned subscription path for xAI.
- Eligibility: reports say some SuperGrok tiers get 403 "do not have an active Grok subscription" [C] ([Hermes docs](https://hermes-agent.nousresearch.com/docs/guides/xai-grok-oauth)).

### 4.3 Enforcement

None found.

---

## 5. Competitor survey

| Product                            | Anthropic                                                                                                                                         | OpenAI                                                                                   | Google                                          | xAI                                                         | Source                                                                                                                                     |
| ---------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------- | ----------------------------------------------- | ----------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------ |
| **OpenCode**                       | Removed in 1.3.0; "Anthropic explicitly prohibits this"                                                                                           | Official SIWC ("Sign in with ChatGPT") plus legacy Codex client id → `backend-api/codex` | Vertex/API key only                             | Grok CLI client id, device code, xAI-endorsed               | [opencode.ai/docs/providers](https://opencode.ai/docs/providers/); repo `packages/core/src/plugin/provider/{chatgpt,openai,xai}.ts` [V]    |
| **OpenClaw**                       | Drives the local `claude` binary ("Claude CLI" runtime), and also accepts pasted `setup-token`s sent with spoofed Claude Code beta headers and UA | SIWC (`extensions/openai/token-sharing.ts`, `dynamic_agent_client`)                      | Stopped offering Gemini CLI / Antigravity OAuth | Grok CLI client id, `cli-chat-proxy`                        | repo files cited above [V]                                                                                                                 |
| **Cline**                          | "Claude Code" provider: spawns the local `claude` CLI per message                                                                                 | n/a checked                                                                              | n/a                                             | n/a                                                         | [docs.cline.bot/provider-config/claude-code](https://docs.cline.bot/provider-config/claude-code) [C, search snippet]                       |
| **Kilo Code**                      | n/a checked                                                                                                                                       | n/a checked                                                                              | n/a                                             | SuperGrok OAuth, xAI-endorsed; not via Kilo's cloud gateway | [x.ai/news/grok-kilocode](https://x.ai/news/grok-kilocode) [V], [kilo.ai/docs/ai-providers/xai](https://kilo.ai/docs/ai-providers/xai) [C] |
| **Zed**                            | Runs Claude Agent from the ACP registry; it "owns its own authentication and billing"                                                             | Codex via ACP, its own login                                                             | Gemini CLI via ACP, its own login               | not listed                                                  | [zed.dev/docs/ai/external-agents](https://zed.dev/docs/ai/external-agents) [V]                                                             |
| **OpenMausBot**                    | Drives Claude Code; user pastes Anthropic sign-in code                                                                                            | Drives Codex; device code                                                                | —                                               | Grok CLI                                                    | [railway template](https://railway.com/deploy/openmausbot-open-source-ai-bot-team-on-claude-code-codex--openmausbot) [C]                   |
| **Tucano**                         | Claude Code CLI with `claude auth login`                                                                                                          | Codex, signed in from inside Tucano                                                      | —                                               | —                                                           | [skillsllm listing](https://skillsllm.com/skill/tucano) [C]                                                                                |
| Roo, Crush, agents.chat, "openbot" | not researched or not found                                                                                                                       |                                                                                          |                                                 |                                                             | —                                                                                                                                          |

Pattern: products that survive enforcement either (1) use a vendor program (OpenAI SIWC, xAI partner endorsement), or (2) shell out to the vendor's own unmodified CLI, which owns the login. Products that impersonated first-party clients against Anthropic and Google were blocked or banned.

---

## 6. Recommendation for Cubebot

Cubebot is open source (Apache-2.0) and self-hosted. A single-person installation where the operator connects their own account is the best case under every vendor's text. Two Cubebot properties raise the risk:

- **Installation-wide connections shared by all agents and all human users** ([identity ADR](../adr/identity.md#installation-managed-provider-connections)). A consumer subscription connected by one person and used by other people's agents is account sharing: Anthropic §2 says "may not make your Account available to anyone else", and OpenAI keys credentials per registration. For subscription connections, Cubebot should mark them **personal**: one owner, used only for that owner's agents and work, and never offered to other users of a multi-user installation. That conflicts with the current "all eligible agents" default; it needs an amendment.
- **Always-on agents.** Usage counts against human-sized plan limits ("ordinary, individual usage"). Surface limit errors (OpenAI `subscription_sharing_usage_limit_exceeded`) as a visible wait under the existing [retry-and-wait](../adr/agents.md#retry-and-wait-for-the-selected-model) decision.

Hosted multi-tenant would change the answer: OpenAI requires the interest form or approval; Anthropic bars intermediating; Google and xAI give no permission. Do not offer subscription login in any hosted Cubebot.

### Per vendor

| Vendor        | Do                                                                                                                                                                                                                                                    | Don't                                                                                            |
| ------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------ |
| **OpenAI**    | (a) Implement SIWC "ChatGPT plan usage" in the native Rust engine. Optionally file the interest form to confirm self-hosted servers qualify                                                                                                           | Don't use Codex's client id or `chatgpt.com/backend-api`                                         |
| **xAI**       | (b-minus) Optional, behind a clear "endorsed for OpenCode/Kilo; Cubebot not yet approved" warning, or better, ask xAI for written OK first. Device-code OAuth fits a server well. The cleanest alternative is driving `grok agent` (ACP) or `grok -p` | Don't assume it bills `api.x.ai`; it goes through the CLI proxy                                  |
| **Anthropic** | Optional "Claude Code runtime": run the unmodified `claude` in the sandbox; the user runs `claude auth login` in a sandbox terminal; Cubebot never reads the credential file. Also promote Max/Ultra/Team monthly API credits through a Console key   | No Cubebot OAuth, no setup-token paste field, no header spoofing, no "Log in with Claude" button |
| **Google**    | Keep AI Studio / Vertex API keys                                                                                                                                                                                                                      | No Gemini CLI or Antigravity OAuth, no `agy -p` wrapping                                         |

### What changes in Cubebot (high level, no code yet)

1. **Connection kind.** Add a `ChatGptPlan` (OAuth) credential kind alongside the API key in `apps/orchestrator/src/connections/` (today `Inference.api_key: String`, `connections/mod.rs` ~L79). Store the issued `oaiapp_…` client id, a stable `ext_agent_host_id` per installation, the access token, the rotating refresh token and the id token, encrypted as today. Make the connection personal (see above).
2. **Sign-in UX.** SIWC requires a `127.0.0.1` loopback redirect in the browser's machine and offers no device code. For a remote Cubebot that means either the documented "complete OAuth locally, then import the credential file" flow (a tiny local helper or CLI), or a "paste the redirected 127.0.0.1 URL" step. The second is common practice but **not documented by OpenAI**; confirm with them.
3. **Refresh.** This fits the existing [orchestrator-owned refresh](../adr/identity.md#orchestrator-owned-provider-authentication-refresh): serialize refreshes per connection (refresh tokens rotate), refresh before the 1 h access-token expiry, propagate only the access token to sandboxes, and keep the refresh token in the orchestrator. Hourly propagation becomes a real path, not an edge case.
4. **Provider adapter.** `modules/providers/src/openai.rs` already POSTs `api.openai.com/v1/responses` with `bearer_auth`, `store:false`, `stream:true`. For the plan route: move the `role:"system"` message to `instructions` (L124), drop `max_output_tokens` (L160) and other unsupported fields, group tools into namespaces if required, list models from `/v1/models` filtered by `visibility=="list"`, and map `subscription_sharing_*` and `response.incomplete` to wait or failure states. The same adapter can mostly be reused with a mode flag.
5. **xAI (if pursued).** Device-code OAuth to `auth.x.ai`; Responses-shaped requests to `cli-chat-proxy.grok.com/v1`; refresh in the orchestrator.
6. **Claude Code runtime (if pursued).** This is a new runtime shape (an external agent loop and tools inside the sandbox, with credentials owned by the binary). It needs an ADR that amends "native Rust agent engine" and "propagated authentication" for this one provider, plus the sandbox image changes. It is the largest change, for the least certain policy footing.

---

## 7. Open questions

1. **OpenAI:** does a self-hosted open-source server that the user reaches remotely count as "remotely hosted" (interest form required) or as the "Self-hosted VMs" case? Is the paste-the-callback-URL pattern acceptable? When does the preview end, and do terms change then?
2. **Anthropic:** does running the unmodified `claude` binary inside a self-hosted Cubebot sandbox require the _self-hoster_ to accept Commercial Terms (the "hosted sandboxes or other agent infrastructure" clause)? Do always-on multi-agent workloads count as "ordinary, individual usage"? What is the status of the paused SDK credit change after Nov 2026? Ask Anthropic sales, as their page suggests.
3. **xAI:** is there, or will there be, a registration path for other open-source apps ("More open-source agents and integrations are coming soon")? What do the consumer ToS say about automated or third-party access? (The page was unreadable to me.)
4. **Google:** does Google AI Pro/Ultra include any Gemini API (AI Studio) credit that a user could use through an API key? Not researched.
5. Competitors not covered: Roo Code, Crush, agents.chat, "openbot" (no primary source found).

## 8. Sources

Anthropic

- https://code.claude.com/docs/en/legal-and-compliance
- https://code.claude.com/docs/en/authentication
- https://code.claude.com/docs/en/agent-sdk/overview
- https://www.anthropic.com/legal/consumer-terms
- https://support.claude.com/en/articles/15036540-use-the-claude-agent-sdk-with-your-claude-plan
- Secondary (timeline only): https://gigazine.net/gsc_news/en/20260220-anthropic-third-party-block , https://falcao.org/posts/anthropic-claude-access-crackdown-ecosystem-fallout/ , https://alternativeto.net/news/2026/4/anthropic-ends-free-claude-access-for-third-party-tools-like-openclaw , https://thenewstack.io/anthropic-pauses-claude-agent-sdk-subscription-change/

OpenAI

- https://developers.openai.com/siwc
- https://developers.openai.com/siwc/token-sharing-open-source
- https://developers.openai.com/siwc/token-sharing-open-source/sign-in
- https://developers.openai.com/siwc/token-sharing-open-source/models-and-inference
- https://developers.openai.com/siwc/token-sharing-open-source/token-reference
- https://developers.openai.com/siwc/token-sharing-open-source/preview-limitations
- https://developers.openai.com/siwc/token-sharing-open-source/profiles-and-sessions
- https://developers.openai.com/siwc/token-sharing-open-source/self-hosted-vms
- https://developers.openai.com/siwc/token-sharing-open-source/codex-app-server
- https://learn.chatgpt.com/docs/sign-in-with-chatgpt
- https://learn.chatgpt.com/docs/auth
- https://help.openai.com/en/articles/20001542-using-your-chatgpt-plan-in-other-apps-and-sites (HTTP 403, not read)
- https://openai.com/form/sign-in-with-chatgpt-interest/
- https://github.com/openai/codex (`codex-rs/tui/src/onboarding/auth.rs`, `codex-rs/cli/src/doctor.rs`)

Google

- https://github.com/google-gemini/gemini-cli/blob/main/docs/resources/tos-privacy.md
- https://github.com/google-gemini/gemini-cli/blob/main/docs/resources/faq.md
- https://github.com/google-gemini/gemini-cli/commit/83a3851d
- https://github.com/google-gemini/gemini-cli/tree/main/packages/core/src/code_assist
- https://developers.google.com/gemini-code-assist/docs/deprecations/code-assist-individuals
- https://antigravity.google/terms
- https://antigravity.google/docs/faq
- https://antigravity.google/docs/cli/headless/
- Community: https://github.com/openclaw/openclaw/issues/14203 , https://discuss.ai.google.dev/t/account-restricted-without-warning-google-ai-ultra-oauth-via-openclaw/122778 , https://discuss.ai.google.dev/t/is-external-orchestration-of-antigravity-cli-headless-mode-supported-with-account-based-usage/183051 , https://discuss.ai.google.dev/t/is-it-allowed-for-an-open-source-tool-to-call-the-official-agy-cli-in-headless-mode/183429

xAI

- https://x.ai/news/grok-opencode
- https://x.ai/news/grok-kilocode
- https://auth.x.ai/.well-known/openid-configuration
- https://github.com/xai-org/grok-build (`docs/user-guide/02-authentication.md`, `14-headless-mode.md`, `15-agent-mode.md`, `scripts/install.sh`)
- https://x.ai/legal/terms-of-service (Cloudflare 403, not read)
- Community: https://hermes-agent.nousresearch.com/docs/guides/xai-grok-oauth

Competitors

- https://opencode.ai/docs/providers/ and https://github.com/anomalyco/opencode (`packages/core/src/plugin/provider/`)
- https://github.com/openclaw/openclaw (`docs/providers/anthropic.md`, `docs/providers/google.md`, `extensions/anthropic/stream-wrappers.ts`, `extensions/openai/token-sharing.ts`, `extensions/xai/`)
- https://zed.dev/docs/ai/external-agents
- https://docs.cline.bot/provider-config/claude-code
- https://kilo.ai/docs/ai-providers/xai
- https://railway.com/deploy/openmausbot-open-source-ai-bot-team-on-claude-code-codex--openmausbot
- https://skillsllm.com/skill/tucano
