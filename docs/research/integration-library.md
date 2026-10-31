# Integration library survey

Research date: October 30, 2026. Question: which external integrations could Cubebot agents use with little work, so they can seed a native integration library? Scope: vendor-maintained MCP servers, official APIs and CLIs, authentication and review requirements, API pricing, and developer terms that restrict automation or AI use. Each one is judged against Cubebot's model: MCP is the default ([ADR 0040](../adr/0040-native-core-tools-and-mcp-integrations.md)), local stdio and remote Streamable HTTP servers ([ADR 0041](../adr/0041-local-and-remote-mcp-servers.md)) are configured by administrators ([ADR 0042](../adr/0042-administrator-managed-integration-configuration.md)), connections are installation-managed only ([ADR 0043](../adr/0043-installation-managed-external-account-connections.md)), and every connection defaults to all agents ([ADR 0081](../adr/0081-installation-wide-default-connection-access.md)).

Method: vendor documentation, pricing and terms pages, and official repositories, read through web fetches on the research date. Some primary pages blocked automated fetching: Reddit, Discord policy, Salesforce, X developer agreement and Bluesky. For those, the note says **secondary** or **unverified**. Nothing was connected, purchased or run. A documented endpoint is not proof that it works with Cubebot's MCP client. Every row below still needs a qualification run before it can be called supported.

Current Cubebot baseline ([docs/STATUS.md](../STATUS.md)): agents already have an MCP client for stdio and Streamable HTTP servers. It is disabled by default, runs inside the sandbox as the tool user, and has no per-tool grants. Integrations, the administrator registry and connection records are **not implemented**. So "MCP config only" below means "once the ADR 0042 registry and secret delivery exist, no integration-specific code".

## Summary

- **The easy tier is large and mostly developer tooling.** GitHub, GitLab, Linear, Atlassian (Jira, Confluence, Jira Service Management, Bitbucket Cloud), Sentry, Cloudflare, Stripe, Supabase, Airtable, Notion (Business and Enterprise plans), Intercom and the search APIs (Exa, Tavily, Firecrawl, Brave) all run vendor-hosted MCP servers. Each accepts a **static bearer credential** an administrator can paste: a PAT, API key, service-account key or agent key. Each of these needs only an MCP config entry and a stored secret.
- **The second tier needs one generic feature, not per-vendor adapters: an MCP OAuth client.** Slack, Google Workspace, HubSpot, Asana, ClickUp, Todoist, Box, Dropbox, Vercel and Salesforce accept **OAuth only** on their hosted MCP servers. Cubebot needs one orchestrator-side MCP authorization flow: dynamic or pre-registered client, admin consent, refresh-token storage and renewal. After that, this whole tier is MCP config too. Some of these vendors also allowlist clients. **Vercel** accepts only clients it has reviewed. **Dropbox** allows dynamic registration only for "a trusted set" of clients. **Asana** clients can be blocked per workspace. **Slack** admits only directory-published or internal apps.
- **Most OAuth MCP servers act as a human user.** HubSpot states that every action follows the installing user's permissions. Linear, Atlassian, Stripe and Notion offer an app identity, a service account or an agent key instead. This choice decides whether an installation-owned connection is really owned by the installation.
- **Cloud CLIs are the cheapest path for infrastructure.** `aws`, `gcloud` and `az` run inside the VM. AWS also runs a GA managed MCP server with SigV4 auth. Google runs managed MCP endpoints for BigQuery, Cloud SQL, GKE and other services.
- **Social is the hard tier, but X is now paid rather than impossible.** X has no free tier. It bills per call: $0.015 per post created, $0.005 per post read, and a cap of 3M reads per month. X also maintains `xmcp`, a local MCP server. **LinkedIn** lets anyone post through Share on LinkedIn (`w_member_social`) without approval. Reading, organization pages and messaging all need partner approval, and its User Agreement bans bots and browser-extension automation. **Reddit** has required approval for new OAuth tokens since November 2025 (secondary sources). **WhatsApp Business** forbids "AI providers" whose main product is AI, so a general-purpose Cubebot assistant over WhatsApp is not viable.
- **Microsoft 365 has no full official MCP server.** Microsoft's own MCP for Graph is a read-only Entra identity preview. App-only Graph with Exchange RBAC for Applications fits installation ownership very well, but it needs a native adapter or a community MCP server. Teams posting by app identity is limited to data migration, so Teams needs a bot.
- **For the ADRs:** the installation-only model fits organizations' tools well. It fits **personal mailboxes and calendars** poorly. If those connections become available to all agents by default (ADR 0081), any agent can read someone's private mail. Recommended change: make mailbox, calendar and personal-social connection types default to explicit grants. Details are under "Implications for ADRs" below.

## Recommended first wave

These are ordered by value per unit of work. Every item works with an administrator-pasted credential and an existing vendor-maintained server or CLI. None of them needs OAuth support in Cubebot first.

| # | Integration | How | Credential an admin stores | Why first |
| --- | --- | --- | --- | --- |
| 1 | GitHub | Remote MCP `https://api.githubcopilot.com/mcp/`, or local `github-mcp-server stdio` | Fine-grained PAT of a machine account, as `Authorization: Bearer` [gh] | Core of coding agents; read-only mode and toolset selection built in |
| 2 | Linear | Remote MCP `https://mcp.linear.app/mcp` (`/mcp/readonly` exists) | Linear API key or OAuth token as a bearer token; can act as an app user [linear] | Issue tracking; app identity avoids a human owner |
| 3 | Atlassian (Jira, Confluence, JSM, Bitbucket Cloud) | Rovo MCP `https://mcp.atlassian.com/v1/mcp` | Service-account API key (Bearer) or personal API token (Basic); org admin must enable API-token auth [atl-token] | One server covers four products |
| 4 | GitLab | Instance MCP at `/api/v4/mcp`, GA since 19.5, Free tier | Fine-grained PAT with MCP permission; legacy PATs are rejected [gitlab] | Works with self-managed GitLab, which suits self-hosters |
| 5 | Sentry | Remote `https://mcp.sentry.dev/mcp` (OAuth), or local `@sentry/mcp-server` with a token [sentry] | Sentry auth token (local stdio) | Debugging loop for coding agents |
| 6 | Cloudflare | `https://mcp.cloudflare.com/mcp` "code mode" (search, execute, docs) | Cloudflare API token as a bearer token, documented for automation [cf] | One server covers the whole Cloudflare API |
| 7 | Stripe | `https://mcp.stripe.com` | **Agent API key** (bearer); plain secret keys stop working in December 2026 [stripe] | Vendor-designed for autonomous agents, with built-in human confirmation on risky writes |
| 8 | Databases: Postgres, Supabase, BigQuery | `psql` in the VM; Supabase MCP `https://mcp.supabase.com/mcp?project_ref=…&read_only=true`; BigQuery `https://bigquery.googleapis.com/mcp` | DB URL, Supabase PAT, or Google identity with IAM roles [supabase] [bq] | Agents query data directly; read-only modes exist |
| 9 | Web search and fetch | Exa `https://mcp.exa.ai/mcp`, Tavily `https://mcp.tavily.com/mcp/`, Firecrawl `https://mcp.firecrawl.dev/v2/mcp`, Brave local server | API key; all have free allowances [exa] [tavily] [firecrawl] [brave] | Agents need search more than anything else on this list |
| 10 | Notion and Airtable | Notion `https://mcp.notion.com/mcp`; Airtable `https://mcp.airtable.com/mcp` | Notion MCP token (Business/Enterprise, workspace owner only) [notion]; Airtable PAT [airtable] | Common knowledge bases and lightweight data stores |
| 10b | AWS, Google Cloud, Azure | `aws`, `gcloud`, `az` CLIs in the VM; AWS MCP `https://aws-mcp.us-east-1.api.aws/mcp` (GA, SigV4) [aws] | IAM keys or role, service-account key, service principal | No adapter needed; the VM already supports installing CLIs |

**Second wave: build one MCP OAuth client, then add these as config.** Slack (internal app), Google Workspace (developer preview), HubSpot, Asana, ClickUp, Todoist, Box, Dropbox, Vercel, Salesforce. Telegram is the easiest **channel** adapter (bot token, free) but needs native inbound plumbing; see Communication.

## Comparison table

Legend for the effort column:

- **MCP config** — a static secret is enough.
- **MCP config (OAuth)** — needs Cubebot's generic MCP OAuth support first, then config.
- **CLI in VM** — the agent installs and uses a vendor CLI.
- **Native adapter** — Cubebot code is required.
- **Not viable** — terms or access block it.

"Admin fit" rates how well the integration suits an installation-owned connection: **good** means a service, bot or app identity exists; **user** means it acts as a human's account; **brand** means one shared brand or social account.

| Integration | Official API | Vendor MCP | Auth for a Cubebot connection | Review or verification | API cost | Terms notes | Admin fit | Effort |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| GitHub | Yes | Remote and local (Go, Docker) [gh] | PAT bearer or OAuth | None for PAT | Free with account | Standard | Good (machine user) | MCP config |
| GitLab | Yes | Remote on each instance, GA 19.5 [gitlab] | Fine-grained PAT or OAuth with DCR | Admin must allow MCP for the group or instance | Free tier; GitLab.com rate limits | Standard | Good | MCP config |
| Bitbucket Cloud | Yes | Through Atlassian Rovo MCP [atl-tools] | API token or OAuth 2.1 | Workspace must be linked to an Atlassian org | Some Rovo tools use credits | Standard | Good | MCP config |
| Linear | Yes | Remote [linear] | API key bearer, app-user OAuth | None | Linear plan | Standard | Good | MCP config |
| Jira, Confluence, JSM | Yes | Remote Rovo MCP [atl-tools] | Service-account API key or user API token; OAuth 2.1 | Org admin enables API-token auth [atl-token] | Some tools use Rovo credits | Standard | Good | MCP config |
| Sentry | Yes | Remote (OAuth) and local npm [sentry] | Token on local server | None | Sentry plan | Standard | Good | MCP config (local) |
| Vercel | Yes | Remote, OAuth [vercel] | OAuth only | **Client must be reviewed and approved by Vercel** | All plans | Vercel AI Product Terms | User | MCP config (OAuth) after allowlisting; else CLI in VM |
| Cloudflare | Yes | Remote, plus product-specific servers [cf] | API token bearer | None | Account plan | Standard | Good | MCP config |
| AWS | Yes | Managed remote MCP, GA May 2026 [aws] | IAM SigV4 through `mcp-proxy-for-aws` | None | No extra charge | Standard | Good (IAM role) | MCP config or CLI in VM |
| Google Cloud | Yes | Managed remote MCP for BigQuery, Cloud SQL, GKE, Cloud Run and more [gmcp] | OAuth with IAM | None | Underlying service costs | Standard | Good (service account) | MCP config (OAuth) or CLI in VM |
| Azure | Yes | Azure MCP Server (local); ARM remote listed [msmcp] | Entra service principal | None | Underlying | Standard | Good | CLI in VM or MCP config |
| Slack | Yes | Remote `https://mcp.slack.com/mcp` [slackmcp] | Confidential OAuth, user token; **no dynamic client registration** | **Only directory-published or internal apps**; unlisted apps prohibited | Free API; non-Marketplace commercial apps limited to 1 rpm on history, internal apps exempt [slackrl] | Marketplace is the only channel for commercial distribution | Good if each installation registers its own internal app | MCP config (OAuth) for tools; native adapter for a channel |
| Discord | Yes | None found | Bot token | `MESSAGE_CONTENT` is a privileged intent; verification needed at 100+ servers [discord] | Free | Developer Policy not fetched (**unverified**) | Good (bot) | Native adapter (channel) |
| Microsoft Teams | Yes (Graph, Bot Framework) | Agent 365 servers need Copilot licensing (**secondary**) [a365] | Bot registration; app-only message posting is **migration only** [teamsmsg] | Tenant admin consent | M365 licenses | Microsoft APIs Terms | Good (bot) | Native adapter |
| Telegram | Yes (Bot API) | None | Bot token from BotFather | None | Free; about 30 msg/s bulk, 20 msg/min per group [tg] | Bots never see other bots' messages | Good (bot) | Native adapter (channel); sending alone is CLI in VM |
| WhatsApp Business | Yes (Cloud API) | None | System-user token | Meta business verification | Per-message pricing (not checked) | **AI Providers clause bans AI-as-main-product** [wa] | n/a | **Not viable** as a general assistant |
| Gmail | Yes | Remote `gmailmcp.googleapis.com` (**Developer Preview**) [gws] | OAuth; IMAP/SMTP with app password | **Restricted scopes need verification, plus a security assessment if data is stored or transmitted on servers** [gmail-scopes] | Free | Google API Services User Data Policy | User | MCP config (OAuth) for Workspace orgs; CLI in VM over IMAP for single users |
| IMAP/SMTP (any provider) | Protocol | n/a | Password or app password | Gmail app passwords need 2-Step Verification and are unavailable for some org accounts [apppw] | Free | Provider ToS | User or shared mailbox | CLI in VM |
| Outlook and Exchange (Graph) | Yes | Read-only Entra MCP preview only [msent] | App-only, scoped to mailboxes with Exchange RBAC for Applications [exorbac] | Tenant admin consent | Included | Microsoft APIs Terms | **Very good** (scoped app identity) | Native adapter or community MCP |
| Google Drive, Docs, Sheets, Slides, Calendar, Chat | Yes | Remote, **Developer Preview** (9 servers) [gws] | OAuth Web client marked "AI-powered agent"; service-account delegation **not allowed** for that client | Sensitive scopes need verification; Testing apps' refresh tokens expire in 7 days [g-refresh] | Free | Google API Services User Data Policy | User | MCP config (OAuth); or `gws` CLI (unofficial Google project) [gwscli] |
| Microsoft 365 files and calendar | Yes (Graph) | Agent 365 servers, licensed (**secondary**) [a365] | App-only or delegated | Admin consent | Licensing | Microsoft APIs Terms | Good with app-only | Native adapter |
| Notion | Yes | Remote; local server deprecated [notion] | OAuth; **MCP token on Business/Enterprise** | None | Notion plan | Standard | Good with MCP token or service account | MCP config |
| Airtable | Yes | Remote [airtable] | PAT or OAuth | None | All plans, no MCP charge | Standard | Good | MCP config |
| Asana | Yes | Remote V2 `https://mcp.asana.com/v2/mcp` [asana] | OAuth | Clients can be blocked per workspace; admins allow them | Asana plan | Standard | User | MCP config (OAuth) |
| Trello | Yes (REST) | Vendor MCP **unverified** [trello-sec] | API key and token | None | Free | Standard | User | CLI in VM over REST |
| ClickUp | Yes | Remote `https://mcp.clickup.com/mcp` [clickup] | **OAuth 2.1 only, no API keys** | None | 100 to 25,000 calls per day by plan | Standard | User | MCP config (OAuth) |
| Todoist | Yes | Remote `https://ai.todoist.net/mcp` and npm `@doist/todoist-ai` [todoist] | OAuth (remote); API token for REST | None | Free | Standard | User | MCP config (OAuth) or CLI in VM |
| X (Twitter) | Yes | `xdevplatform/xmcp` local server [xmcp] | OAuth 1.0a or 2.0 user token plus bearer | Developer account | **Pay per use: $0.015/post, $0.005/read, 3M reads/month cap** [xprice] | Explicit consent before automated replies or DMs; no bulk actions [xpolicy] | Brand | Native adapter or local MCP; paid |
| LinkedIn | Yes | None | Member OAuth | `w_member_social` (post, comment, like) is self-serve; everything else needs partner approval [li-access] | Free | **User Agreement 8.2 bans bots and automation extensions** [li-ua] | Brand (posting only) | Native adapter for posting; reading not viable |
| Reddit | Yes | None | OAuth | **Approval for new tokens since November 2025** (**secondary**) [reddit-sec] | Commercial use priced separately (**unverified**) | Responsible Builder Policy (primary blocked) | Brand | Hard: request approval first |
| YouTube | Yes | None | OAuth for writes, API key for reads | Quota increase by form | Free; 10,000 units/day default [yt] | YouTube API ToS (not reviewed) | Brand | CLI in VM over REST |
| Instagram and Facebook | Yes (Graph) | None | Professional account; Standard Access for own accounts | **App Review and business verification only for accounts you don't own or manage** [ig] | Free | Platform Terms | Brand (own account) | Native adapter or CLI in VM over REST |
| Bluesky | Yes (AT Protocol) | None | App password or OAuth | None | Free | Docs not readable on research date (**unverified**) | Brand | CLI in VM over REST |
| Mastodon | Yes | None | Register app per instance through the API [masto] | None (instance rules vary) | Free | Per-instance rules | Brand | CLI in VM over REST |
| HubSpot | Yes | Remote `https://mcp.hubspot.com` [hubspot] | OAuth + PKCE through an "MCP connector"; **follows the user's permissions** | None | Some tools need Marketing Hub Pro+ | Some tools limited to named AI clients | User | MCP config (OAuth) |
| Salesforce | Yes | Hosted MCP GA April 2026 (**secondary**) [sf] | OAuth + PKCE through an External Client App | Admin enables each server | Enterprise Edition+ (**secondary**) | Standard | User; service accounts reportedly unsupported (**secondary**) | MCP config (OAuth) |
| Stripe | Yes | Remote [stripe] | **Agent API key** or OAuth | None | Free | Built-in human confirmation on refunds and money movement | Good | MCP config |
| Shopify | Yes | Dev MCP (docs only, local npx); store access through Shopify CLI [shopify] | Custom-app credentials (details **unverified**) | None for custom apps | Free | Standard | Good | CLI in VM |
| Zendesk | Yes | General-purpose server not public as of September 2026 (**secondary**) [zendesk-sec] | API token | None | Zendesk plan | Standard | Good | CLI in VM over REST |
| Intercom | Yes | Remote US and EU [intercom] | OAuth or bearer access token | None | Intercom plan | Standard | Good | MCP config |
| PostgreSQL | Protocol | Reference server **archived** (**secondary**) [pg-sec]; Google MCP Toolbox [gmcp] | Connection URL | n/a | n/a | n/a | Good | CLI in VM (`psql`) or MCP config (Toolbox) |
| BigQuery | Yes | Remote `https://bigquery.googleapis.com/mcp` [bq] | Google identity with IAM roles | None | Query costs | Standard | Good | MCP config (OAuth) or CLI in VM (`bq`) |
| Snowflake | Yes | Snowflake-managed MCP, GA November 2025 [snow] | Snowflake auth (details not reviewed) | None | Compute credits | Standard | Good | MCP config (auth **unverified**) |
| Supabase | Yes | Remote [supabase] | PAT for CI; OAuth with DCR | None | Supabase plan | Docs warn about production data and prompt injection | Good | MCP config |
| Brave Search | Yes | Local `brave/brave-search-mcp-server` [brave] | API key | None | $5 per 1,000 requests, $5 monthly credit | Storing results needs a plan with storage rights | Good | MCP config |
| Exa | Yes | Remote [exa] | API key, OAuth or keyless | None | Free keyless rate limit; signup credits | Standard | Good | MCP config |
| Tavily | Yes | Remote and npm [tavily] | API key | None | 1,000 free credits/month; $0.008/credit [tavily-price] | Standard | Good | MCP config |
| Firecrawl | Yes | Remote [firecrawl] | API key bearer | None | 1,000 free credits/month; $19/month for 5,000 [fc-price] | Target sites' terms still apply | Good | MCP config |
| Dropbox | Yes | Remote `https://mcp.dropbox.com/mcp`, beta [dropbox] | OAuth; **DCR only for listed clients**, others need a manual app | Team admin may restrict apps | Free | Standard | User | MCP config (OAuth) with a manual app |
| Box | Yes | Remote `https://mcp.box.com` [box] | OAuth with admin-issued integration credentials | Admin enables MCP | Plan not stated | Standard | User | MCP config (OAuth) |
| Amazon S3 | Yes | Through AWS MCP [aws] | IAM | None | Storage costs | Standard | Good | CLI in VM |
| Zapier MCP | Aggregator | Remote [zapier] | Zapier account | None | **2 tasks per tool call** from the plan quota | Zapier terms | Good (one Zapier account) | MCP config (OAuth); paid |
| Composio | Aggregator | Remote | API key | None | Free: 100K calls/month (20K on Composio-managed apps); Pro $29/month + usage [composio] | Hosted only; self-hosting not listed | Good | MCP config; paid, external dependency |

## Notes by category

### Code and development

GitHub's remote server is hosted by GitHub and accepts a PAT as a bearer token. The local server is the same Go program run with `stdio`. Read-only mode skips write tools and overrides explicit tool requests. GitHub Enterprise Server is not supported by the remote server, so a GHES installation needs the local server with `--gh-host`. [gh]

GitLab's MCP server became GA in 19.5 and has been on the Free tier since 19.2. It is served by each instance at `/api/v4/mcp`, including self-managed instances. An administrator must allow MCP per top-level group or per instance. Legacy personal, project and group tokens are rejected; a fine-grained PAT with the MCP permission works. It accepts protocol versions 2025-03-26 through 2025-11-25. [gitlab]

Atlassian's Rovo MCP server covers Jira, Confluence, JSM, Bitbucket Cloud, Loom and several Atlassian platform objects. It accepts OAuth 2.1 or API tokens; Teams and Code search are OAuth-only. Atlassian recommends API tokens only for machine-to-machine use, and an organization admin must enable them. Service-account API keys use Bearer auth, which is the cleanest fit for an installation-owned connection. Some Confluence and search tools consume Rovo credits, and `delete_jira`/`manage_jira` are disabled by default. [atl-tools] [atl-token]

Linear's server supports a Linear API key as a bearer token, can act as an `app` user, and has a read-only path. [linear] Sentry hosts `https://mcp.sentry.dev/mcp` with OAuth and also publishes a local npm server that takes a token; the local option is from Sentry's docs as summarized by search. [sentry]

Vercel's MCP is OAuth-only and accepts only "AI clients that have been reviewed and approved by Vercel". Cubebot would have to apply through Vercel's client review form; until then, use the Vercel CLI in the VM. [vercel] Cloudflare's main MCP is a code-mode server that runs generated code in a sandbox, with an API token documented for automation. [cf]

Cloud providers: AWS's managed MCP server became GA on May 6, 2026, runs in us-east-1 and eu-central-1, and uses SigV4 through a local proxy at no extra charge (**AWS What's New, summarized by search**). [aws] Google runs managed remote MCP servers for BigQuery, Cloud SQL, AlloyDB, Spanner, GKE, Compute Engine, Cloud Run (GA) and others. Google also publishes open-source servers: MCP Toolbox for Databases and a gcloud MCP. [gmcp] Microsoft's `microsoft/mcp` repository builds the Azure MCP Server (local) and lists remote servers such as Azure Resource Manager. [msmcp] For an agent in its own VM, the CLIs (`aws`, `gcloud`, `az`) are the lowest-effort path, and their credentials are ordinary environment secrets.

### Communication

These split into two different needs. **Tools** let an agent search or post on request; MCP covers that. **Channels** let people talk to the agent and it answers; that needs inbound events (webhooks, sockets, long polling) delivered to Cubebot's rooms, which no MCP tool server provides. STATUS lists rooms and integrations as not implemented, so channels are native work regardless.

- **Slack:** the official MCP server is Streamable HTTP with confidential OAuth and user tokens. It does not support dynamic client registration. Only directory-published or internal apps may use it. [slackmcp] Since May 2025, commercially distributed apps outside the Marketplace are limited to 1 request per minute and 15 objects per page on `conversations.history`/`replies`. Internal customer-built apps are exempt. [slackrl] For a self-hosted Cubebot, each installation creating its **own internal Slack app** is both allowed and unthrottled. A shared Cubebot-published app would need Marketplace approval.
- **Discord:** bot token. `MESSAGE_CONTENT` is privileged; apps in 100+ servers must be verified and approved for it. [discord] The Developer Policy page returned 403 and is **unverified**.
- **Microsoft Teams:** Graph can post channel and chat messages only with delegated permissions. App-only posting (`Teamwork.Migrate.All`) is for migration. A Cubebot channel therefore needs a Teams bot. [teamsmsg] Microsoft's Agent 365 MCP servers for Outlook and Teams reportedly need Agent 365 and Microsoft 365 Copilot licenses. That claim comes from secondary sources and Microsoft community pages and is **unverified**. [a365]
- **Telegram:** bot token from BotFather, no cost. Limits: about one message per second per chat, 20 messages per minute per group, about 30 per second for bulk sends. In privacy mode a bot sees only commands and replies. [tg] It is the cheapest channel to build.
- **WhatsApp Business:** Section 4.7 "AI Providers" of Meta's WhatsApp Business Platform terms (page updated September 23, 2026) strictly forbids AI or ML technology providers from using the platform when AI is the main product. It also forbids letting platform data train AI models, except fine-tuning a model for the business's own exclusive use. [wa] Press coverage dates enforcement for existing users to January 15, 2026 (**secondary**). Cubebot as a general-purpose agent over WhatsApp is **not viable**. A business narrowly using it for its own support may be allowed; that needs legal review.
- **Email:** Gmail's `readonly`, `compose`, `modify`, `metadata` and full-access scopes are **restricted**. Restricted scopes need OAuth app verification, plus a security assessment if the data is stored on or transmitted through servers. `gmail.send` is only sensitive. [gmail-scopes] An external app left in Testing status gets refresh tokens that expire after 7 days. [g-refresh] Google's Workspace MCP servers (including Gmail) are a **Developer Preview** and need a Web OAuth client marked "Use this client for an AI-powered agent"; service-account domain-wide delegation cannot use that option. [gws] For a single person on consumer Gmail, IMAP/SMTP with an app password is the pragmatic route. App passwords need 2-Step Verification and are unavailable for some organization and Advanced Protection accounts; Google recommends against them. [apppw] Whether internal-only Workspace apps skip verification was not confirmed from the fetched page (**unverified**). [g-verify]
- **Outlook and Exchange:** Microsoft's MCP Server for Enterprise is a read-only Entra identity preview with no mail tools. [msent] App-only Graph with **RBAC for Applications in Exchange Online** can scope `Mail.Read`, `Mail.Send` or `Calendars.ReadWrite` to chosen mailboxes. This is the best match found for "installation-owned connection to specific mailboxes". [exorbac] It needs a native adapter, or a community Graph MCP server run locally and vetted first.

### Productivity

- **Google Workspace:** Google lists nine remote MCP servers (Gmail, Drive, Docs, Sheets, Slides, Calendar, Chat, People, Classroom), all in a Developer Preview program. [gws] Google also hosts the `googleworkspace/cli` (`gws`) repository. Its README says it is "not an officially supported Google product" and expects breaking changes before v1. It can run as an MCP stdio server (**search summary of the README**). [gwscli]
- **Notion:** hosted server only; the open-source local server is "no longer actively maintained". OAuth works on all plans. A **bearer MCP token** for unattended use exists on Business and Enterprise plans and only workspace owners can create one. For shared organizational connections Notion recommends a service account. [notion]
- **Airtable:** remote server on all plans, no MCP charge, PAT or OAuth, with read-only scope sets documented. [airtable]
- **Asana:** V2 server at `/v2/mcp`. The V1 endpoint is deprecated with a shutdown date written "05/11/2026", which is ambiguous between May 11 and November 5. OAuth only. Enterprise admins allow or block clients; other tiers must contact support. [asana]
- **ClickUp:** OAuth 2.1 with PKCE only; personal API tokens are refused. Rolling 24-hour limits run from 100 calls (Free) to 25,000 (Enterprise Plus) per workspace per client. [clickup]
- **Todoist:** official remote MCP and an npm package from Doist. [todoist]
- **Trello:** search found no vendor documentation for a Trello MCP server. One secondary blog claims `mcp.trello.com` launched in July 2026 (**unverified**). The REST API with key and token is simple enough for an agent to use with `curl` in the VM. [trello-sec]
- **Microsoft 365 (OneDrive, SharePoint, Calendar):** same picture as Outlook: Graph app-only plus a native adapter, or licensed Agent 365 servers.

### Social

Social accounts are almost always **one shared brand account**. That fits installation ownership, as long as the connection is not exposed to every agent by default.

- **X:** pay-per-use credits, no subscription. Prices: $0.005 per post read, $0.001 per "owned read", $0.015 per standard post created, $0.200 per post created with a URL. Plans cap at 3M post reads per month, with an Enterprise form above that. There is no free tier; promotional credits exist. [xprice] The automation policy requires explicit consent before automated replies or DMs and bans bulk or aggressive actions. [xpolicy] `xdevplatform/xmcp` exposes the OpenAPI spec as tools on a local server. [xmcp] Verdict: **technically easy and cheap for low-volume posting**. Note that a post containing a URL costs 13× a plain post. Not "not viable", but paid and policy-bound.
- **LinkedIn:** the only self-serve permissions are Sign In with OpenID Connect and **Share on LinkedIn (`w_member_social`: post, comment, like as a member)**. The Advertising API (which includes organization-page management), sales, talent and compliance products need approval; compliance is closed. [li-access] The User Agreement (effective November 3, 2025), §8.2, bans software and browser plugins that scrape, and "bots or other unauthorized automated methods". [li-ua] Verdict: **posting as one member is viable; reading feeds, messages or search is not viable**. ADR 0038 browser automation does not get around this.
- **Reddit:** primary policy pages blocked fetching. Secondary reports say a Responsible Builder Policy (November 2025) requires approval before issuing new OAuth tokens, with reports of quick rejections. [reddit-sec] Verdict: **hard; apply first and do not build ahead of approval**.
- **YouTube:** the Data API gives each project 10,000 units per day by default, with separate allowances for `search.list` and `videos.insert`; more quota is requested by form. [yt] Usable through REST from the VM.
- **Instagram and Facebook:** professional (business or creator) accounts only. Standard Access is enough for accounts you own or manage, or people with roles on the app. App Review plus business verification apply only when serving accounts you don't own. [ig] [meta-review] A self-hosted installation posting to its own account therefore avoids App Review.
- **Bluesky and Mastodon:** open protocols with no approval gate. Mastodon apps register per instance through the API. [masto] Bluesky docs did not render on the research date (**unverified**).

### CRM and business

- **Stripe:** the best-designed agent integration found. It offers agent API keys tagged as such, OAuth for interactive use, and Dashboard-side **human confirmation** for refunds and money movement. Full-access and untagged restricted keys stop working on MCP in December 2026. [stripe]
- **HubSpot:** OAuth with PKCE through an MCP connector the account creates. Every action follows the installing user's permissions, and there is no private-app or service-key path. Some tools are limited to named AI clients. [hubspot] For installation ownership, use a dedicated HubSpot user.
- **Salesforce:** hosted MCP servers reportedly GA April 29, 2026, through External Client Apps with OAuth and PKCE, Enterprise Edition and above. A secondary source says there is no service-account support. Salesforce pages returned 403 (**secondary**). [sf]
- **Shopify:** Dev MCP is a docs and schema server that needs no authentication. Store operations go through the Shopify CLI. [shopify]
- **Zendesk:** a general-purpose MCP server was announced for early access in summer 2026 but had no public endpoint as of early September (**secondary, single source**). [zendesk-sec] The REST API with an API token works from the VM.
- **Intercom:** remote US and EU servers; bearer access tokens supported, but they cannot use the Fin Operator tools. [intercom]

### Data

- **PostgreSQL:** the MCP reference Postgres server was archived in 2025; secondary sources report an unpatched SQL-injection issue in its read-only mode. [pg-sec] Prefer `psql` in the VM with a read-only role, or Google's MCP Toolbox for Databases. [gmcp]
- **Supabase:** remote MCP with `project_ref`, `read_only` and `features` URL parameters. A PAT is the documented CI credential. The docs warn explicitly about production data and prompt injection. [supabase]
- **BigQuery:** remote endpoint enabled together with the BigQuery API. It uses IAM roles and recommends a separate identity per agent. [bq]
- **Snowflake:** managed MCP server GA since November 4, 2025; the docs warn about expensive recursive agent loops. [snow]

### Search and web

All four take an API key and have free allowances. **Exa** even works keyless at a rate limit, but only over a direct IPv4 connection. [exa] **Tavily**: 1,000 free credits per month. [tavily-price] **Firecrawl**: 1,000 free credits per month. [fc-price] **Brave**: $5 per 1,000 requests with $5 in monthly credit; storing results needs a plan with storage rights. [brave] Cubebot already has a native browser planned (ADR 0040), but a search API is cheaper and more reliable for discovery.

### Files and storage

- **Dropbox:** remote server; dynamic client registration only for listed clients. Other clients must create a Dropbox app with the listed scopes. [dropbox]
- **Box:** remote server; an admin enables MCP and issues integration credentials. The self-hosted server is deprecated. [box]
- **S3:** `aws s3` in the VM or the AWS MCP server. [aws]

### Aggregators

- **Zapier MCP:** reaches 9,000+ apps through one connection. Each tool call costs **two tasks** from the plan quota. Credentials stay in Zapier; Enterprise admins control which apps and actions are available. [zapier]
- **Composio:** free tier of 100K tool calls per month; calls through Composio-managed OAuth apps count against a 20K allowance. Pro is $29/month plus usage. Self-hosting is not listed. [composio]
- **Assessment:** useful as an **escape hatch for the long tail** (CRM-adjacent SaaS, niche tools) and to skip OAuth app registration. Costs: a third party holds every credential, which cuts against a self-hosted product's main selling point. It is another bill. And audit and identity become the aggregator's. Recommend supporting them as ordinary remote MCP endpoints, not as the library's foundation. Pipedream was not reviewed.

## Hard or not viable

| Integration | Verdict | Reason |
| --- | --- | --- |
| WhatsApp Business as an agent channel | Not viable | Meta's terms §4.7 bans AI providers whose main product is AI, and bans training AI on platform data [wa] |
| LinkedIn reading (feed, messages, search, profiles) | Not viable | Needs partner programs; User Agreement §8.2 bans bots and automation extensions, so browser automation is no workaround [li-access] [li-ua] |
| LinkedIn organization-page posting | Hard | Advertising or Community Management API approval [li-access] |
| Reddit | Hard | New OAuth tokens need approval since November 2025; anecdotal denials (**secondary**) [reddit-sec] |
| X at volume | Hard on cost | More than 3M reads/month goes to Enterprise; URL posts cost $0.20 each [xprice] |
| Gmail API, public multi-tenant OAuth app | Hard | Restricted scope verification plus security assessment [gmail-scopes]. Each self-hosted installation using its own Google project avoids the public-app path. |
| Microsoft Teams messages from an app identity | Hard | App-only send is migration-only; needs a bot [teamsmsg] |
| Vercel MCP | Blocked until approved | Client allowlist review [vercel] |
| Slack app shared across installations | Hard | Needs Marketplace approval; per-installation internal apps avoid it [slackmcp] [slackrl] |
| Browser automation of any of the above | Avoid | LinkedIn bans it explicitly; others not reviewed here |

## Implications for ADRs 0040, 0043 and 0081

1. **ADR 0040/0041 hold up.** The vendor ecosystem has converged on remote Streamable HTTP MCP. Every vendor checked that offers MCP offers it remotely. Native adapters are needed only for **channels** (Slack, Discord, Teams, Telegram events) and for **Microsoft Graph**.
2. **The biggest missing piece is generic MCP authorization.** At least ten valuable servers are OAuth-only. Building them in requires:
   - OAuth 2.1 with PKCE and both dynamic and pre-registered clients. Slack and Dropbox reject dynamic registration for unknown clients. Google needs a Web client per installation.
   - An administrator consent flow in the console.
   - Refresh-token storage and renewal on the orchestrator. Agents must never hold refresh tokens.
   - Delivery of short-lived access tokens to the VM.

   This is one feature, not ten adapters. It should be designed before the second wave. The ADR 0041 open item "authentication and secret storage" is where it belongs.
3. **Client allowlists hit Cubebot as a product, not per installation.** Vercel, Dropbox and Asana vet MCP clients. Each Cubebot installation is its own OAuth client. So either administrators register their own OAuth apps (Dropbox manual app, Slack internal app, Google Web client), or the project applies for allowlisting under a shared identity. That shared identity in turn conflicts with self-hosting.
4. **ADR 0043 (installation-only) fits organizational tools.** Service, app or bot identities exist for GitHub, GitLab, Atlassian, Linear, Stripe, Notion, Cloudflare, AWS, GCP, Graph app-only and chat bots. Several OAuth-only servers (HubSpot, ClickUp, Asana, Google Workspace MCP, Dropbox) **act as the human who consented**. Under 0043 the record is installation-owned, but the external identity is a person's. Administrators should create dedicated service users. The connection UI should show which external identity a connection acts as.
5. **Personal accounts are the real gap.** "My inbox", "my calendar" and "my LinkedIn" are personal data. ADR 0043 lets an administrator connect them; ADR 0081 then makes them available to all agents by default. Options to decide:
   - (a) Default explicit-grants for connection types flagged personal (mail, calendar, personal social), keeping installation ownership.
   - (b) Revisit a user-owned connection category.
   - (c) Document that single-user installations are the intended home for personal accounts.

   Option (a) is the smallest change consistent with both ADRs.
6. **Scope at the source beats Cubebot-side filtering.** Many servers offer read-only or scoped variants:
   - GitHub `--read-only` and toolsets
   - Linear `/mcp/readonly`
   - Supabase `read_only` and `project_ref`
   - GitLab toolset headers
   - Exchange RBAC mailbox scopes
   - Stripe agent-key permissions

   ADR 0044's tool allowlists should treat upstream scope as the primary control. They should record which variant a connection uses, because MCP tools currently have no per-tool grants.
7. **Prompt-injection risk is the vendors' own warning.** Supabase, Stripe and Vercel all warn about it. Stripe's server-side confirmation is a model worth copying in Cubebot's approval design. Nothing here is a reason to treat a successful tool call as qualification.

## Open questions

- Will Cubebot implement MCP OAuth (client registration, consent UI, refresh on the orchestrator) before any integration ships, or launch with bearer-token servers only?
- Should Cubebot apply to client allowlists (Vercel, Dropbox) under a project identity, or always require per-installation OAuth apps?
- Personal-account policy: explicit grants by default for mailbox and calendar types, or a user-owned connection category?
- Channels: Telegram first (cheapest), or Slack (the most likely team use)? Either one is native work in the not-yet-built rooms module.
- How should the library pin and vet local MCP packages (`@sentry/mcp-server`, `xmcp`, the Brave server) under ADR 0042's executable-provenance rule?
- Exact Asana V1 shutdown date ("05/11/2026"); Trello and Zendesk official MCP status; Salesforce editions and service-account support; Reddit's current policy text; Agent 365 licensing. All need a primary-source recheck.
- Do installation-internal Google Workspace OAuth apps skip verification for restricted Gmail scopes? Unconfirmed here.

## Sources

Fetched or searched October 30, 2026. Dates in brackets are page dates where the page showed one. "Secondary" marks a source that is not the vendor's own.

[gh]: https://github.com/github/github-mcp-server
[gitlab]: https://docs.gitlab.com/user/model_context_protocol/mcp_server/
[linear]: https://linear.app/docs/mcp
[atl-tools]: https://developer.atlassian.com/cloud/rovo-mcp/guides/supported-tools/
[atl-token]: https://developer.atlassian.com/cloud/rovo-mcp/guides/configuring-authentication-via-api-token/
[sentry]: https://docs.sentry.io/product/sentry-mcp/
[vercel]: https://vercel.com/docs/agent-resources/vercel-mcp
[cf]: https://developers.cloudflare.com/agents/model-context-protocol/mcp-servers-for-cloudflare/
[aws]: https://aws.amazon.com/about-aws/whats-new/2026/05/aws-mcp-server/
[gmcp]: https://github.com/google/mcp
[msmcp]: https://github.com/microsoft/mcp
[slackmcp]: https://docs.slack.dev/ai/mcp-server/
[slackrl]: https://docs.slack.dev/changelog/2025/05/29/rate-limit-changes-for-non-marketplace-apps
[discord]: https://docs.discord.com/developers/events/gateway
[teamsmsg]: https://learn.microsoft.com/en-us/graph/api/chatmessage-post
[a365]: https://learn.microsoft.com/en-us/microsoft-copilot-studio/use-work-iq
[tg]: https://core.telegram.org/bots/faq
[wa]: https://www.facebook.com/legal/Meta-Terms-for-WhatsApp-Business-Platform
[gmail-scopes]: https://developers.google.com/workspace/gmail/api/auth/scopes
[g-refresh]: https://developers.google.com/identity/protocols/oauth2
[g-verify]: https://support.google.com/cloud/answer/13463073
[apppw]: https://support.google.com/accounts/answer/185833
[gws]: https://developers.google.com/workspace/guides/configure-mcp-servers
[gwscli]: https://github.com/googleworkspace/cli
[msent]: https://learn.microsoft.com/en-us/graph/mcp-server/overview
[exorbac]: https://learn.microsoft.com/en-us/exchange/permissions-exo/application-rbac
[notion]: https://developers.notion.com/guides/mcp/get-started-with-mcp
[airtable]: https://support.airtable.com/docs/using-the-airtable-mcp-server
[asana]: https://developers.asana.com/docs/using-asanas-mcp-server
[clickup]: https://developer.clickup.com/docs/connect-an-ai-assistant-to-clickups-mcp-server
[todoist]: https://github.com/Doist/todoist-mcp
[trello-sec]: https://www.scalekit.com/blog/trello-mcp-vs-api
[xprice]: https://docs.x.com/x-api/getting-started/pricing
[xpolicy]: https://docs.x.com/developer-terms/agreement-and-policy
[xmcp]: https://github.com/xdevplatform/xmcp
[li-access]: https://learn.microsoft.com/en-us/linkedin/shared/authentication/getting-access
[li-ua]: https://www.linkedin.com/legal/user-agreement
[reddit-sec]: https://r.datuan.dev/r/redditdev/comments/1oug31u/introducing_the_responsible_builder_policy_new/noc31sg
[yt]: https://developers.google.com/youtube/v3/getting-started
[ig]: https://developers.facebook.com/docs/instagram-platform/overview
[meta-review]: https://developers.facebook.com/docs/resp-plat-initiatives/individual-processes/app-review
[masto]: https://docs.joinmastodon.org/client/token/
[hubspot]: https://developers.hubspot.com/docs/apps/developer-platform/build-apps/integrate-with-the-remote-hubspot-mcp-server
[sf]: https://developer.salesforce.com/blogs/2026/04/salesforce-hosted-mcp-servers-are-now-generally-available
[stripe]: https://docs.stripe.com/mcp
[shopify]: https://shopify.dev/docs/apps/build/devmcp
[zendesk-sec]: https://tldv.io/blog/zendesk-mcp/
[intercom]: https://developers.intercom.com/docs/guides/mcp
[pg-sec]: https://startdebugging.net/2026/08/migrate-off-archived-mcp-reference-servers/
[bq]: https://docs.cloud.google.com/bigquery/docs/use-bigquery-mcp
[snow]: https://docs.snowflake.com/release-notes/2025/other/2025-11-04-cortex-agents-mcp
[supabase]: https://supabase.com/docs/guides/getting-started/mcp
[brave]: https://brave.com/search/api/
[exa]: https://docs.exa.ai/reference/exa-mcp
[tavily]: https://docs.tavily.com/documentation/mcp
[tavily-price]: https://www.tavily.com/pricing
[firecrawl]: https://docs.firecrawl.dev/mcp-server
[fc-price]: https://www.firecrawl.dev/pricing
[dropbox]: https://help.dropbox.com/integrations/connect-dropbox-mcp-server
[box]: https://developer.box.com/guides/box-mcp
[zapier]: https://zapier.com/mcp
[composio]: https://composio.dev/pricing

| Key | Source | Kind and date |
| --- | --- | --- |
| gh | [github/github-mcp-server README][gh] | Vendor repo |
| gitlab | [GitLab MCP server docs][gitlab] | Vendor docs, version history to 19.6 |
| linear | [Linear MCP docs][linear] | Vendor docs |
| atl-tools, atl-token | [Rovo MCP supported tools][atl-tools] (updated Oct 27, 2026); [API token auth][atl-token] (updated Jun 30, 2026) | Vendor docs |
| sentry | [Sentry MCP docs][sentry] | Vendor docs, read through a search summary |
| vercel | [Vercel MCP][vercel] | Vendor docs, updated Sep 15, 2026 |
| cf | [Cloudflare MCP servers][cf] | Vendor docs |
| aws | [AWS What's New: AWS MCP Server GA][aws] | Vendor announcement, May 2026; read through a search summary |
| gmcp, bq | [google/mcp][gmcp]; [BigQuery MCP][bq] | Vendor repo and docs |
| msmcp, msent | [microsoft/mcp][msmcp]; [MCP Server for Enterprise][msent] (updated Jul 4, 2026) | Vendor repo and docs |
| exorbac | [RBAC for Applications in Exchange Online][exorbac] (updated Oct 27, 2026) | Vendor docs |
| teamsmsg | [Graph chatMessage post][teamsmsg] | Vendor docs |
| a365 | [Work IQ MCP overview][a365] | Vendor docs; licensing claims came from secondary search results (**unverified**) |
| slackmcp, slackrl | [Slack MCP server][slackmcp]; [rate-limit changelog][slackrl] (May 29, 2025) | Vendor docs |
| discord | [Discord Gateway docs][discord] | Vendor docs |
| tg | [Telegram Bots FAQ][tg] | Vendor docs |
| wa | [Meta Terms for WhatsApp Business Platform][wa] (updated Sep 23, 2026; served in Portuguese) | Vendor terms |
| gmail-scopes, g-refresh, g-verify, apppw, gws | Google [Gmail scopes][gmail-scopes], [OAuth 2.0][g-refresh], [verification help][g-verify], [app passwords][apppw], [Workspace MCP setup][gws] | Vendor docs |
| gwscli | [googleworkspace/cli][gwscli] | Repo under the Google Workspace org; README says not an officially supported product |
| notion, airtable, asana, clickup, todoist | [Notion][notion]; [Airtable][airtable]; [Asana][asana] (updated Feb 20, 2026); [ClickUp][clickup]; [Doist/todoist-mcp][todoist] | Vendor docs and repo |
| xprice, xpolicy, xmcp | [X API pricing][xprice]; [developer policy][xpolicy]; [xdevplatform/xmcp][xmcp] | Vendor docs and repo; the policy page shows no date |
| li-access, li-ua | [LinkedIn getting access][li-access] (updated Jun 3, 2026); [User Agreement][li-ua] (effective Nov 3, 2025) | Vendor docs and terms |
| yt, ig, meta-review, masto | [YouTube getting started][yt]; [Instagram Platform][ig]; [Meta App Review][meta-review]; [Mastodon tokens][masto] | Vendor docs |
| hubspot, stripe, shopify, intercom | [HubSpot][hubspot]; [Stripe MCP][stripe]; [Shopify Dev MCP][shopify]; [Intercom MCP][intercom] | Vendor docs |
| sf | [Salesforce GA blog][sf] | Vendor blog, Apr 2026; returned 403, details from secondary search results (**secondary**) |
| supabase, snow | [Supabase MCP][supabase]; [Snowflake release note][snow] (Nov 4, 2025) | Vendor docs |
| brave, exa, tavily, tavily-price, firecrawl, fc-price | [Brave API][brave]; [Exa MCP][exa]; [Tavily MCP][tavily]; [Tavily pricing][tavily-price]; [Firecrawl MCP][firecrawl]; [Firecrawl pricing][fc-price] | Vendor docs and pricing |
| dropbox, box | [Dropbox help][dropbox]; [Box MCP][box] | Vendor docs |
| zapier, composio | [Zapier MCP][zapier]; [Composio pricing][composio] | Vendor pages |
| reddit-sec | [r/redditdev announcement thread, mirror][reddit-sec] (Nov 11, 2025) | **Secondary**; Reddit's own policy pages blocked fetching |
| trello-sec | [Scalekit blog][trello-sec] | **Secondary** |
| zendesk-sec | [tl;dv blog][zendesk-sec] (about Sep 2026) | **Secondary**, single author |
| pg-sec | [startdebugging.net][pg-sec] (Aug 2026) | **Secondary** |
