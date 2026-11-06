# Native connections: research and proposal

Researched 2026-11-05. D1–D4 were decided by the user on 2026-11-05 (see the end), and the design is accepted as [Native connectors](../adr/identity.md#native-connectors).

## What already exists

- [Installation-managed external account connections](../adr/identity.md#installation-managed-external-account-connections) is accepted. Connections are installation resources that administrators manage. Several accounts can sit on one integration, and a missing account is never silently swapped for another. Its _Open details_ list is mostly what this proposal fills in.
- [Installation-wide connection access by default](../adr/identity.md#installation-wide-connection-access-by-default) already provides the access model you asked for: all agents by default, or specific grants. MCP servers, providers and vault items use the same `Access` type.
- [Autonomous tool use within granted permissions](../adr/agents.md#autonomous-tool-use-within-granted-permissions) means there is no approval queue. The grant is the only safety control, so a connection's grant has to say what an agent may do (read, write, send), not just whether it can use the connection.
- OAuth is mostly built already. `connections/chatgpt.rs` and `subscriptions.rs` implement PKCE, the "open this URL, then paste the `127.0.0.1` address back" loopback flow, token refresh owned by the orchestrator, and short-lived keys sent to the agent with `api_key_expires_at`. `oauth2` and `openidconnect` are in the lockfile.
- MCP secrets reach the agent in `Turn.mcp_servers`, and the agent runs the calls in its own process. Tools are named `server__tool` and found through tool search, so a large catalog does not fill up the context window.
- Routines can already be triggered by webhooks (`0052_webhook_deliveries`). Connection events fit into that path.

## The proposal in one paragraph

A **connector** is compiled-in code that implements one trait. A **connection** is a row: connector + account + sealed credential + access + permission level. The orchestrator owns the catalog, the sign-in flows, token refresh, health checks and event polling. The agent runs the tool calls using a short-lived credential. Each connector declares how it authenticates. It never implements OAuth itself. Adding an integration means writing one file and adding one line to a registry.

## The interface

A new shared crate, `modules/connectors`, is needed. The orchestrator uses it for the catalog, sign-in, health checks and polling, and the agent uses it to run tools. That makes it shared code under the AGENTS.md rule, but AGENTS.md currently lists only `protocol` and `providers`, so adding it needs an ADR amendment.

```rust
pub trait Connector: Send + Sync {
    /// Static: id, name, category, icon, setup guide, auth methods, config fields, tools.
    fn manifest(&self) -> &'static Manifest;
    /// "Who am I": proves the credential works and names the account ("victor@gmail.com").
    async fn check(&self, cx: &Context) -> Result<AccountInfo, ConnectorError>;
    async fn call(&self, cx: &Context, tool: &str, input: Value) -> Result<Value, ConnectorError>;
    /// Optional: new events since `cursor` (new mail, event starting soon, issue assigned).
    async fn poll(&self, _cx: &Context, _cursor: Option<&str>) -> Result<Poll, ConnectorError> {
        Ok(Poll::unsupported())
    }
}

pub enum Auth {
    ApiKey { fields: &'static [Field] },              // Notion, Linear, Todoist, Home Assistant
    Password { fields: &'static [Field] },            // IMAP/SMTP, CalDAV, Nextcloud app passwords
    OAuth2 { authorize: &'static str, token: &'static str, scopes: &'static [Scope],
             device: Option<&'static str>, byo_client: bool },  // Google, Microsoft, Slack, Dropbox
    GitHubAppManifest,                                 // one-click app creation, see below
}

pub struct ToolSpec { name, description, schema, risk: Risk }
pub enum Risk { Read, Write, Send }  // Send = irreversible or reaches other people: email, chat post, payment
```

`Context` gives the connector an HTTP client that is already authenticated (base URL, auth header, retry on 429) along with the account's config. A typical REST connector then comes to 100–300 lines of tool definitions and JSON mapping.

Shared contract tests run against every registered connector: valid manifest, unique tool names, valid JSON schemas, a risk declared on every tool, and a `check` test against recorded HTTP fixtures. The point is that a new contributor learns the rules from failing tests rather than from a review comment.

### Where calls run

**Decided (D1): the agent runs calls, using short-lived credentials, and the credentials are opaque to the model.** Refresh tokens and client secrets never leave the orchestrator. The agent gets only an access token (Google and Microsoft tokens last about an hour) and fetches a new one when it expires, the same way subscriptions work today. This keeps the AGENTS.md rule that the agent runs its own tools.

Caveat: API keys and app passwords do not expire, so they reach the sandbox just as MCP secrets already do. A prompt-injected agent with `bash` could exfiltrate them.

**Opaque means two things.** The model never sees a token: it calls `gmail-work__search`, and no tool result, journal entry or ledger row carries the credential. The agent runtime also fetches and refreshes tokens itself (`GET /api/runtime/connections/{id}/credential`) and runs connector calls inside its own process, not as a child under the unprivileged tool user. So `bash` cannot read them from an environment variable or a file the way it can read a stdio MCP server's secrets today. A prompt-injected agent can still _misuse_ the tools it is granted, but it cannot walk off with the refresh token or the API key.

Rejected: the orchestrator proxying every call. It would make the orchestrator a tool runner, which AGENTS.md rules out.

### Grants

Grants keep the existing access setting (all agents or specific agents) and add a **permission level** to each connection: _Read_, _Read and write_, or _Full_ (includes `Send`). The agent only receives tools at or below the level. With no approval queue, this is what stops an injected email from making the agent email your contacts. You can also switch off individual tools, as ADR "Tool allowlists" expects.

### Naming and multiple accounts

A connection has a slug, and its tools are named like MCP tools: `gmail-work__search`, `gmail-personal__send`. This meets the ADR rule that two accounts on one integration stay distinguishable. Tool search keeps 300 tools out of the prompt.

### Events

`poll()` turns connections into routine triggers: "when mail arrives from my bank…", "10 minutes before any meeting…". The orchestrator polls, because a self-hosted install usually has no public URL. Vendor webhooks can come later through the existing webhook path, for installs that do have one.

## Self-hosted authentication: the real constraints

There is no Cubebot-registered app with any vendor, so every connector needs a path that works for a single person running their own copy. In order of setup effort:

| Path                                                                         | Effort    | Who supports it                                                                                                                                                                                                                                                                                                                                                               |
| ---------------------------------------------------------------------------- | --------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Paste an API key or token                                                    | Seconds   | Notion, Linear, Todoist, Home Assistant, GitHub fine-grained PAT, Jira/Confluence, Airtable, HubSpot private app, Stripe restricted key, Telegram/Discord bots, Cal.com                                                                                                                                                                                                       |
| Username + app password                                                      | 1 min     | IMAP/SMTP (Gmail, iCloud, Fastmail, Yahoo, Outlook, self-hosted), CalDAV/CardDAV, Nextcloud                                                                                                                                                                                                                                                                                   |
| Remote MCP with OAuth client registration (MCP servers tab, not Connections) | One click | Linear, Notion, Atlassian, Sentry and others. Needs OAuth in our MCP client. The MCP spec now prefers CIMD, which needs a public HTTPS metadata URL, with DCR as a fallback; vendor support varies ([MCP OAuth is broken](https://bindify-marketing-prod.pages.dev/blog/mcp-oauth-broken), [den.dev on the auth spec](https://den.dev/blog/mcp-november-authorization-spec/)) |
| App created from a manifest                                                  | 2 clicks  | GitHub App manifest flow: we send the user to GitHub with a manifest, GitHub creates the app on _their_ account and returns its ID, key and secret ([GitHub docs](https://docs.github.com/enterprise-server@2.21/apps/building-github-apps/creating-github-apps-from-a-manifest)). Slack accepts an app manifest the same way, after which the user pastes the bot token      |
| Bring-your-own OAuth app + device code                                       | 5 min     | Microsoft 365: an Entra app registration with "Allow public client flows" turned on. No secret and no redirect URL, so it works behind NAT ([walkthrough](https://github.com/JeremyTBradshaw/MSGraphPSEssentials/wiki/Get-an-access-token-via-Device-Code-flow))                                                                                                              |
| Bring-your-own OAuth app + loopback paste                                    | 10 min    | Google, Dropbox, Spotify, Zoom. Uses the flow `chatgpt.rs` already has, so no public URL is needed. If the install has public HTTPS, a normal callback replaces the paste                                                                                                                                                                                                     |

### Google specifics (what users will run into)

- **Desktop app** OAuth client type plus the loopback-paste flow works on any install. Google retired the copy-the-code (OOB) flow, and its device flow does not allow Gmail or Calendar scopes.
- **Testing mode issues refresh tokens that expire after 7 days.** The fix is to set the consent screen to _In production_. An unverified app then shows a warning screen and is capped at 100 users, which doesn't matter for personal use ([Google health setup](https://developers.google.com/health/setup), [unverified apps](https://support.google.com/cloud/answer/7454865)). A Google Workspace user can choose _Internal_ and avoid both problems. Not confirmed: whether Gmail's restricted scopes on an unverified production app need anything more for a single user. Test this before writing the guide.
- One Google connection should cover Gmail, Calendar, Drive, Contacts, Docs and Sheets. The user ticks the products they want, and we request only those scopes.
- Google now runs official remote MCP servers for Gmail, Calendar and Drive (Preview), and they also need the user's own OAuth client ([Google guide](https://developers.google.com/workspace/guides/configure-mcp-servers)). They are a cheap option to evaluate, but they're in Preview and their Gmail tools appear to stop at drafts. Native tools are the safer base.
- The card's setup guide must be a wizard: numbered steps, deep links into Cloud Console, a scope list to copy, the exact redirect URI to copy, and a "test connection" button that runs `check()`.

## Email

Build **one Email connector over IMAP/SMTP** rather than one per provider. It works with Gmail (app password or XOAUTH2), Outlook, iCloud, Fastmail, Yahoo, Proton (through Bridge) and any self-hosted server. When a Google or Microsoft connection exists, Email can sign in with XOAUTH2 using that connection's token, so the user doesn't need an app password. Gmail's IMAP extensions expose threads and labels. Not yet in the lockfile: `async-imap`, `lettre`, `mail-parser`.

Tools: `search`, `read_thread`, `list_folders`, `draft`, `reply`, `send` (Send), `move`/`label` (Write), and `save_attachment` into the agent's Files. Two rules:

1. Mail content is untrusted input, and tool results are marked as external content. **Decided (D2): sending is on by default.** A new Email connection gets the _Full_ level, and the administrator can lower it or switch off `send` on the connection.
2. `poll()` provides the "new mail" trigger. That is what turns an inbox into something the agent works on, not just something it can search.

## Calendar and Meet

Calendar comes through the Google and Microsoft connections, plus **CalDAV** for iCloud, Fastmail, Nextcloud and Radicale. The tools you'd expect: free/busy, list, create, respond. `poll()` provides "a meeting starts in N minutes".

Joining Google Meet is _not_ something a connection can do: no Meet API lets a bot join the call's audio and video for real. The realistic path is: a calendar event triggers a routine → the agent opens the Meet link in its own browser, signed in from the vault → the voice-calls speech pipeline carries the audio. The connection provides the event and the link. Joining is a desktop/voice project.

## Catalog, prioritized for usefulness

**Wave 1: build the foundation and prove every auth kind with one connector each.**

| Connector                                              | Why                                                   | Auth                   |
| ------------------------------------------------------ | ----------------------------------------------------- | ---------------------- |
| Email (IMAP/SMTP)                                      | The highest-value personal-assistant data             | Password / XOAUTH2     |
| Google (Gmail, Calendar, Drive, Contacts, Sheets/Docs) | Most common personal and work suite                   | BYO OAuth, loopback    |
| Microsoft 365 (Outlook, Calendar, OneDrive, To Do)     | The other half of the market                          | BYO OAuth, device code |
| CalDAV / CardDAV                                       | iCloud, Fastmail, Nextcloud; the self-hosting crowd   | Password               |
| GitHub                                                 | Cubebot users are technical                           | App manifest / PAT     |
| Notion                                                 | Notes and wikis                                       | API key                |
| Slack                                                  | Acting in the user's workspace                        | Manifest + token       |
| Telegram bot                                           | Lets you reach your agents from your phone            | Token                  |
| HTTP API (generic)                                     | Anything else: base URL + auth header + allowed paths | Key                    |

**Wave 2 (built 2026-11-05): mostly token-paste connectors, each one quick to build.**

| Connector                           | Sign-in                                                    | Default level | Events                              |
| ----------------------------------- | ---------------------------------------------------------- | ------------- | ----------------------------------- |
| Linear                              | API key                                                    | Full          | issue assigned, comment mentions me |
| Jira + Confluence (Atlassian Cloud) | Email + API token, site URL                                | Write         | issue assigned                      |
| Todoist                             | API token                                                  | Write         | task due                            |
| Trello                              | API key + token                                            | Write         |                                     |
| Asana                               | Personal access token                                      | Write         | task assigned                       |
| Airtable                            | Personal access token, base list                           | Write         |                                     |
| Home Assistant                      | URL + long-lived token                                     | Write         | watched entity changes              |
| Nextcloud Files                     | URL + app password (WebDAV)                                | Write         |                                     |
| Paperless-ngx                       | URL + token                                                | Write         |                                     |
| Discord bot                         | Bot token                                                  | Full          | message mentions the bot            |
| Dropbox                             | The person's own app, OAuth PKCE                           | Write         |                                     |
| Cal.com                             | API key, base URL for self-hosted                          | Write         | booking created, cancelled          |
| Calendly                            | Personal access token                                      | Read          | invitee created                     |
| Zoom                                | The person's own Server-to-Server app (client credentials) | Read          | recording ready                     |
| HubSpot                             | Service key or legacy private app token                    | Write         |                                     |
| Stripe                              | Restricted key; refunds are Send                           | Read          | payment succeeded, failed           |
| Readwise                            | Token                                                      | Read          |                                     |
| RSS / Atom                          | None; feed list                                            | Read          | new item                            |

Two sign-ins were added for them, once and generically: the client-credentials grant (Zoom) and no credential (RSS).

**Not in the catalog (D3):** vendor remote-MCP servers stay in Platform › MCP servers. OAuth support for remote MCP servers is still worth doing, but as an improvement to that tab, not as Connections cards.

**Avoid:** WhatsApp personal accounts (against the terms, accounts get banned), the X and LinkedIn APIs (paid or restricted; the browser plus vault already covers them), and bank aggregators (Plaid needs a registered app).

## Console

- **Platform › Connections** tab, next to MCP servers: a search box, category filter chips (Mail and calendar, Files, Dev, Work, Chat, Home, Finance), and a grid of cards. Each card has a logo, a name, a one-line description and a status chip: _Not connected_, _Connected as victor@…_, _Needs sign-in again_, or _Error: last check 2h ago_. A connector with two accounts shows a count.
- Clicking a card opens a sheet with the setup wizard, accounts (add/remove, "test connection"), access (all agents / specific), permission level, tool toggles, events, and the last check result.
- The agent's **Connections** tab (the placeholder in your screenshot) lists the connections that agent can use, with their level, using the same pattern as the MCP list above it.

## Phased plan

1. **ADR + crate.** Amend the identity and agents ADRs to answer the external-account _Open details_, and accept `modules/connectors`. Write the trait, registry, contract tests, `connections` table (connector, slug, config, sealed credential, access, level, status, cursor) and the generic OAuth/device/paste flows lifted out of `subscriptions.rs`.
2. **Delivery.** Add `Turn.connections` with short-lived credentials and a refresh endpoint, run tools in the agent, and add the Platform tab and the agent tab.
3. **Wave 1 connectors**, in this order: HTTP API and Notion (the simplest, to prove the trait), Email, Google, Microsoft, CalDAV, GitHub, Slack, Telegram.
4. **Events:** `poll()` → routine triggers.
5. Wave 2 connectors (done 2026-11-05). Remote-MCP OAuth is separate work on the MCP servers tab.

## Decisions (user, 2026-11-05)

- **D1, where calls run:** in the agent, with short-lived credentials the runtime fetches and refreshes itself. Credentials are opaque to the model and kept out of the tool user's reach.
- **D2, email sending:** agents can send email by default. The administrator can switch `send` off, or lower the level, on each connection.
- **D3, MCP cards:** no. MCP servers belong only in the MCP servers list.
- **D4, Cubebot-owned client IDs:** no. Every OAuth connector uses an app the user creates on their own account.
