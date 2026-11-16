import { connectors } from './connectors'

export const repo = 'https://github.com/vicotrbb/cubebot'
export const sponsor = 'https://github.com/sponsors/vicotrbb'
export const install = 'curl -fsSL https://cubebot.io/install.sh | bash'
export const version = 'v0.3.6'
export const released = '2026-11-16'

export const docs = {
  manifesto: `${repo}/blob/main/MANIFESTO.md`,
  architecture: `${repo}/blob/main/ARCHITECTURE.md`,
  adrs: `${repo}/tree/main/docs/adr`,
  deploy: `${repo}/blob/main/deploy/README.md`,
  reference: `${repo}/blob/main/docs/reference.md`,
  contributing: `${repo}/blob/main/CONTRIBUTING.md`,
  license: `${repo}/blob/main/LICENSE`,
  issues: `${repo}/issues`,
  releases: `${repo}/releases`,
  chart: 'oci://ghcr.io/vicotrbb/charts/cubebot',
}

export const resources = [
  {
    href: '/download/',
    title: 'Download',
    body: 'Every way to install, and what each one needs.',
  },
  {
    href: '/docs/install/',
    title: 'Install guide',
    body: 'The installer, step by step, and the cubebot command.',
  },
  {
    href: '/docs/features/',
    title: 'Feature tour',
    body: 'Everything an agent and the console can do today.',
  },
  {
    href: '/docs/architecture/',
    title: 'Architecture',
    body: 'Orchestrator, sandboxes, journal and the database.',
  },
  {
    href: '/docs/security/',
    title: 'Security',
    body: 'The trust boundary, credentials, and what is qualified.',
  },
  {
    href: '/changelog/',
    title: 'Changelog',
    body: 'Every release, newest first.',
  },
] as const

export const status = {
  today: [
    'Each agent in its own KVM guest or declared container',
    'One-command install, Docker Compose or Helm',
    'OpenAI, Anthropic, Gemini, Grok, OpenRouter and MiniMax',
    'Use a ChatGPT, SuperGrok or MiniMax M Plan',
    'One permanent conversation per agent',
    'Terminal, files, browser and a desktop you can take over',
    `${connectors.length} native connectors and MCP servers`,
    'Rooms, delegation, tasks and parallel lanes',
    'Memory, dreaming and a shared skill library',
    'Knowledge base, shared files and a vault',
    'Routines on schedules, webhooks and account events',
    'Voice calls, with speech that runs offline',
    'Agent teams with managers, shared instructions and an org chart',
    'Groups, scoped grants, OpenID Connect, audit log',
    'Usage tracking, daily and monthly budgets per agent',
    'Profiles agents read: who you are and how you work',
    'Backups to S3-compatible storage, restore into a new install',
    'A calendar with your events and every routine run',
    'Agents that set up agents, rooms and teams, with your approval',
    'A model of its own for each agent',
    'An org chart that fits, pans and zooms',
    'A title for each agent on its cards',
  ],
  progress: [
    'Real-cluster fencing and isolation',
    'Network policy on production CNIs',
    'Scale at 100 registered, 25 working agents',
    'Live checks for every connector',
  ],
  planned: [
    'Hybrid memory retrieval',
    'Skill imports from files and Git',
    'Push notifications',
    'Vendor webhooks for connectors',
  ],
}

export const timeline = [
  {
    date: 'Oct 24',
    datetime: '2026-10-24',
    title: 'Design baseline',
    body: 'Manifesto, architecture, first decisions.',
  },
  {
    date: 'Oct 28',
    datetime: '2026-10-28',
    title: 'Its own sandbox',
    body: 'An in-house VMM, one KVM guest per agent.',
  },
  {
    date: 'Oct 31',
    datetime: '2026-10-31',
    title: 'Memory and rooms',
    body: 'Dreams, skills, a desktop, delegation.',
  },
  {
    date: 'Nov 3',
    datetime: '2026-11-03',
    title: 'First release',
    body: 'v0.1.0 and the one-command install.',
  },
  {
    date: 'Nov 5',
    datetime: '2026-11-05',
    title: 'Tasks and voice',
    body: 'Parallel tasks, knowledge base, calls.',
  },
  {
    date: 'Nov 6',
    datetime: '2026-11-06',
    title: '27 connectors',
    body: 'Your accounts, at the level you set.',
  },
  {
    date: 'Nov 8',
    datetime: '2026-11-08',
    title: 'Agent teams',
    body: 'Managers, roles and an org chart.',
  },
  {
    date: 'Nov 9',
    datetime: '2026-11-09',
    title: 'Calendar',
    body: 'Your events and agent routines together.',
  },
  {
    date: 'Nov 10',
    datetime: '2026-11-10',
    title: 'Agents that build',
    body: 'Ask an agent to set up your platform.',
  },
  {
    date: 'Nov 15',
    datetime: '2026-11-15',
    title: 'Meetings',
    body: 'Video, voice and screens, with your agents.',
  },
] as const

export const next = [
  { title: 'Video calls', body: 'Agents join your meetings.' },
  { title: 'Kanban', body: 'Tasks on a board you share with agents.' },
] as const

export const releases = [
  {
    tag: 'v0.3.6',
    date: '2026-11-16',
    title: 'See how each agent computer is doing',
    items: [
      'The Status tab shows CPU, memory, storage and network for the agent computer, and the desktop view has an optional overlay with the same readings',
      'Dreaming condenses harder: it merges memories by topic, drops resolved and one-off detail, and keeps each memory short',
      'Importing passwords from Firefox works when several logins share a site',
    ],
  },
  {
    tag: 'v0.3.5',
    date: '2026-11-16',
    title: 'Agents answer sooner in meetings and calls',
    items: [
      'Agents reuse their prompt cache from turn to turn, so a reply starts sooner and costs less in every conversation',
      'On a call or in a meeting agents think briefly and answer in two or three short sentences',
      'An agent says it is checking before it looks something up, so the room is not left in silence',
    ],
  },
  {
    tag: 'v0.3.4',
    date: '2026-11-16',
    title: 'Agents speak in meetings again, and in Portuguese',
    items: [
      'An agent voice from ElevenLabs or OpenAI no longer stops after its first sentence and leaves every agent in the meeting thinking',
      'A voice that fails tells the room and gives up the floor, so the next agent can still answer',
      'Agents wait for you to finish a sentence before they answer, and a speaking agent shows that you can talk to interrupt it',
      'A speech language setting lets the built-in engines hear and speak Brazilian Portuguese',
    ],
  },
  {
    tag: 'v0.3.3',
    date: '2026-11-16',
    title: 'Agents speak in meetings, and the public host serves only meetings',
    items: [
      'Agents in a meeting take the floor and answer aloud again; the floor decision no longer runs out of room before it answers',
      'ElevenLabs keys that cannot list voices, or are on a free plan, fall back to default voices that every plan can use',
      'When an agent cannot speak, the room shows why instead of staying silent',
      'On its own public address, Cubebot now answers only meeting links, their files and the public meeting API, whatever the proxy in front routes to it',
      'Plain HTTP on the public meetings address redirects to HTTPS, which browsers then remember',
    ],
  },
  {
    tag: 'v0.3.2',
    date: '2026-11-15',
    title: 'Fixes for agents, calls and meeting links',
    items: [
      'Agents whose sandbox the node refused to start, as can happen while a node restarts, now restart instead of staying stuck',
      'ElevenLabs calls speak with a voice your account has, and a refused request now says why',
      'Meeting links published through a Cloudflare tunnel work: the public route uses only path types the tunnel controller supports',
      'The message box sits at the bottom of every chat again',
    ],
  },
  {
    tag: 'v0.3.1',
    date: '2026-11-15',
    title: 'Meetings with your agents',
    items: [
      'Start a meeting now or schedule one, with video, voice and screen sharing, from the New menu, the Calendar or the new Meetings page',
      'Invite people, agents and any email address, also while the meeting runs; invitations go out through your own Gmail, Outlook or email connection',
      'Agents hear everyone, answer when named or clearly addressed, speak in their own voice, can share their desktop, and keep the transcript and a summary in their chat',
      'Share a link anyone can open: guests wait in a lobby until someone lets them in, and nothing a guest says gives an agent its tools',
      'Kubernetes installs can publish the meeting links alone on a second, public ingress and keep the console private',
    ],
  },
  {
    tag: 'v0.3.0',
    date: '2026-11-15',
    title: 'Reasoning effort, password import and a security pass',
    items: [
      'Set how hard agents think: a default per AI provider and a level of its own for any agent, sent the way each vendor documents',
      'Import the passwords Chrome, Edge, Brave, Firefox or Safari saved into the vault, read in your browser and never uploaded',
      'Rename connections, and give them underscores, so one connector can hold several accounts such as google_work and google_personal',
      'Knowledge base pages pick from about 300 searchable emoji',
      'Agents type a saved login only into its own site, private rooms stay private on the situation and activity screens, and pages no longer load outside images',
    ],
  },
  {
    tag: 'v0.2.16',
    date: '2026-11-15',
    title: 'Microsoft 365 sign-in fixed',
    items: [
      'Connecting Microsoft 365 works again: sign-in no longer fails with "the vendor could not be reached"',
      "When a vendor refuses a sign-in, the message now includes the vendor's own reason, such as Microsoft's AADSTS code",
      'The Microsoft 365 setup guide covers account types, the tenant, admin consent and the device-code sign-in',
    ],
  },
  {
    tag: 'v0.2.15',
    date: '2026-11-14',
    title: 'Up-to-date dependencies',
    items: [
      'The WebSocket library behind agent connections and the desktop view moves to its latest release',
      'Build and release tooling is updated, with no change to how Cubebot runs',
      'The repository now has a security policy, a code of conduct and issue templates',
    ],
  },
  {
    tag: 'v0.2.14',
    date: '2026-11-13',
    title: 'The console fits your phone',
    items: [
      'The installed app clears the iPhone status bar and notch',
      'Text fields no longer zoom the page when you tap them on a phone',
      'A room header keeps its member names on one line, and the composer hint sits under Send',
    ],
  },
  {
    tag: 'v0.2.13',
    date: '2026-11-11',
    title: 'Files in chat, and agents that ship',
    items: [
      'Attach images and files to messages in chats and rooms by picking, pasting or dropping them; agents see the images',
      '@mentions of agents and people are highlighted as you type and in the conversation',
      'The calendar opens faster: provider reads reuse connections and nearby weeks load ahead',
      'GitHub agents push files to a branch and merge pull requests, on by default for new connections',
    ],
  },
  {
    tag: 'v0.2.12',
    date: '2026-11-10',
    title: 'Approvals that stay quiet',
    items: [
      'Approving an agent’s requests no longer sends it a message or wakes it',
      'An agent hears back only when a request is declined or cannot be made',
    ],
  },
  {
    tag: 'v0.2.11',
    date: '2026-11-10',
    title: 'Agents that come back after a reboot',
    items: [
      'Agents lost when their node reboots restart instead of showing Thinking forever',
      'A smaller memory request on KVM now really runs a smaller guest',
    ],
  },
  {
    tag: 'v0.2.10',
    date: '2026-11-10',
    title: 'Agents with titles',
    items: [
      'Give an agent a title, such as Research lead, in its About section',
      'The title shows under the agent’s name on its hover card and on org chart cards',
    ],
  },
  {
    tag: 'v0.2.9',
    date: '2026-11-10',
    title: 'Quieter approvals, safer bursts',
    items: [
      'Approve or decline an agent’s requests right on the turn that asked, or approve them all at once',
      'An agent hears every outcome of a turn in one message, instead of one message per approval',
      'Agents can ask to rename another agent or change its cube, with your approval, and give new agents a cube of their own',
      'The org chart fits on first view, then pans and zooms with drag, pinch, scroll or keys',
      'Administrators can delete rooms; history stays readable for administrators',
      'KVM agents reserve the memory their virtual machine really uses, so a node is never overcommitted, and agents that do not fit show as waiting for room',
      'Agents created from requests boot on their first message, and at most five new agents wait for approval at once',
    ],
  },
  {
    tag: 'v0.2.8',
    date: '2026-11-10',
    title: 'A model for each agent',
    items: [
      'Pick each agent’s own model on its Usage tab, from the provider in use or any provider with a saved key; the platform default stays one click away',
      'Platform settings open with an About section: version, deployment, sandbox, default model, uptime, database and counts',
      'General, Speech, Tasks and Backup settings use the whole page, with each section explained beside its controls',
      'Rooms and teams can start empty; add agents and people later from one searchable member picker',
      'People join rooms and team rooms, and retired agents are never offered as members',
      'A WHOOP connector reads recovery, sleep, strain and workouts through your own WHOOP app',
      'The message box shrinks back after you send a long message',
    ],
  },
  {
    tag: 'v0.2.7',
    date: '2026-11-10',
    title: 'Agents that build',
    items: [
      'Ask an agent to create agents, rooms and teams; each request waits for your approval in its conversation',
      'Approving creates it as you, with your access, the default skills and your connection defaults',
      'The agent hears what you decided, including why a request could not be created',
      'New default skills teach agents to design a focused agent, a room or a team before asking',
      'A per-agent switch turns the capability off',
    ],
  },
  {
    tag: 'v0.2.6',
    date: '2026-11-09',
    title: 'Calendar',
    items: [
      'A calendar with your Google, Microsoft 365 and CalDAV events beside every routine run, past and planned',
      'Create, edit and delete events and routines from the calendar, one occurrence or the whole series',
      'Routines run in their own time zone and keep their time across daylight saving',
      'Repeating CalDAV events expand with their exceptions and moved occurrences',
      'An iOS-friendly home-screen icon and name',
    ],
  },
  {
    tag: 'v0.2.5',
    date: '2026-11-08',
    title: 'Agent teams, Home and backups',
    items: [
      'Agent teams: a manager, shared instructions, a role for each member and a room of their own',
      'Managers run their teams, and a task can go to a whole team',
      'Home shows every agent at a glance, as a list or an org chart',
      'Backups to S3-compatible storage without pausing agents, restored into a new install',
      'Access teams for people are now called groups',
    ],
  },
  {
    tag: 'v0.2.4',
    date: '2026-11-07',
    title: 'ChatGPT plan fixes and real brand marks',
    items: [
      'ChatGPT plan replies that ended without their output no longer fail',
      'Streaming replies arrive smoothly instead of in half-second bursts',
      'A + beside Agents and Rooms in the sidebar to add one',
      'Connections show each service’s real logo',
    ],
  },
  {
    tag: 'v0.2.3',
    date: '2026-11-06',
    title: 'Data, notifications, MiniMax and profiles',
    items: [
      'SQL databases, S3-compatible storage and SFTP as connections',
      'ntfy, Pushover and Gotify notifications',
      'MiniMax with a pay-as-you-go key or a personal M Plan key',
      'Profiles agents read, and daily budgets beside monthly ones',
      'More cube paints, eyes and accessories, and any color you pick',
      'A clearer agent Access tab; retired agents move to Health',
    ],
  },
  {
    tag: 'v0.2.2',
    date: '2026-11-06',
    title: 'Anthropic models work with every agent tool',
    items: [
      "Agent requests stay within Anthropic's limits on strict tools",
      "A refused provider request logs the provider's reason",
    ],
  },
  {
    tag: 'v0.2.1',
    date: '2026-11-06',
    title: 'Agents start on clusters with a default priority class',
    items: [
      'Agent pods accept the priority a cluster-wide default PriorityClass gives them',
      'The orchestrator logs why an agent could not start',
    ],
  },
  {
    tag: 'v0.2.0',
    date: '2026-11-06',
    title: 'Ready for exposed clusters',
    items: [
      'Agents reach the orchestrator on their own listener, never through the public ingress',
      'Installation settings and single sign-on move into the console',
      'A hardened Helm chart, published with every release',
      'A new website with the integrations catalog and resources',
    ],
  },
  {
    tag: 'v0.1.15',
    date: '2026-11-06',
    title: 'Wave 2 connectors',
    items: [
      'Linear, Jira and Confluence, Asana, Todoist, Trello, Airtable and HubSpot',
      'Discord, Readwise, RSS and Atom, and Calendly',
      'Home Assistant, Nextcloud Files and Paperless-ngx',
      'Dropbox, Cal.com, Zoom and Stripe',
      'Sign-in with client credentials or none at all; uploads from the workspace',
      'Private destinations for self-hosted servers in the Helm chart',
    ],
  },
  {
    tag: 'v0.1.14',
    date: '2026-11-06',
    title: 'Native connections',
    items: [
      'Platform › Connections and each agent’s connected accounts',
      'Email, Google, Microsoft 365, CalDAV and CardDAV, GitHub, Notion, Slack, Telegram and a generic HTTP API',
      'Read, Read and write, and Full permission levels',
      'Account events start routines',
    ],
  },
  {
    tag: 'v0.1.13',
    date: '2026-11-05',
    title: 'Plan sign-ins',
    items: [
      'Sign in with a ChatGPT Plus or Pro plan',
      'Opt-in SuperGrok sign-in, with its account-risk warning',
    ],
  },
  {
    tag: 'v0.1.12',
    date: '2026-11-05',
    title: 'Console polish',
    items: ['Accessibility, touch targets and states across the console'],
  },
  {
    tag: 'v0.1.11',
    date: '2026-11-05',
    title: 'Production readiness',
    items: [
      'Ownership fix for a dead orchestrator’s claim and bounded shutdown',
      'Audit fixes, Open Graph image and SEO',
    ],
  },
  {
    tag: 'v0.1.10',
    date: '2026-11-05',
    title: 'Voice calls',
    items: [
      'Call an agent over its normal conversation',
      'Offline speech on the server, or OpenAI and ElevenLabs',
    ],
  },
  {
    tag: 'v0.1.9',
    date: '2026-11-04',
    title: 'Task lanes',
    items: [
      'Tasks run side by side and spread across agents',
      'The orchestrator stops speaking for agents',
    ],
  },
  {
    tag: 'v0.1.8',
    date: '2026-11-04',
    title: 'Tasks',
    items: [
      'Tasks: one request, the orchestrator picks agents and splits the work',
      'Gemini and Grok providers',
      'Drawn previews and code rendering in chat',
      'A simpler Teams screen',
    ],
  },
  {
    tag: 'v0.1.7',
    date: '2026-11-04',
    title: 'Situation room',
    items: [
      'Chat rendering and a file viewer',
      'Agent skills tab, usage budgets and the Situation room',
    ],
  },
  {
    tag: 'v0.1.6',
    date: '2026-11-04',
    title: 'Knowledge base',
    items: [
      'A shared knowledge base for people and agents',
      'OpenRouter provider and an agent hover card',
    ],
  },
  {
    tag: 'v0.1.5',
    date: '2026-11-04',
    title: 'Broadcast and audit',
    items: [
      'Broadcast to every agent',
      'Audit log, profile pictures and a vault login backstop',
    ],
  },
  {
    tag: 'v0.1.4',
    date: '2026-11-04',
    title: 'Streams and sandboxes',
    items: [
      'Event streams open at once in Firefox',
      'Sandboxes are replaced on a new image',
    ],
  },
  {
    tag: 'v0.1.3',
    date: '2026-11-03',
    title: 'The agent’s computer',
    items: ['A usable desktop for every agent', 'MCP servers on the platform'],
  },
  {
    tag: 'v0.1.2',
    date: '2026-11-03',
    title: 'Always on',
    items: [
      'Agents stay on and sleep while idle',
      'Console menu and screen fixes',
    ],
  },
  {
    tag: 'v0.1.1',
    date: '2026-11-03',
    title: 'Interactive installer',
    items: [
      'The four-question installer and cubebot uninstall',
      'The chart takes existing Secrets',
    ],
  },
  {
    tag: 'v0.1.0',
    date: '2026-11-03',
    title: 'First release',
    items: [
      'Install with Docker Compose from a release',
      'The console, routines and a vault',
    ],
  },
] as const

// Brand marks for the wall's "What it can reach" tiles (keys of modules/brand/logos.ts).
export const wallLogos: Record<string, string> = {
  Gmail: 'gmail',
  GitHub: 'github',
  Slack: 'slack',
  Linear: 'linear',
  Notion: 'notion',
  'Google Calendar': 'google',
  Jira: 'atlassian',
  'Home Assistant': 'home-assistant',
  Stripe: 'stripe',
  Outlook: 'microsoft',
  Discord: 'discord',
  Nextcloud: 'nextcloud',
  Zoom: 'zoom',
  'PostgreSQL and MySQL': 'sql',
  'S3 buckets': 's3',
  'ntfy and Pushover': 'ntfy',
}

export const wall = [
  {
    label: 'What an agent can do',
    tone: 'orange',
    items: [
      'Run a terminal',
      'Edit files',
      'Browse with Chromium',
      'Use a real desktop',
      'Remember',
      'Dream while idle',
      'Write its own skills',
      'Fill passwords it never sees',
      'Delegate to other agents',
      'Run on a schedule',
      'Answer a webhook',
      'Take a phone call',
    ],
  },
  {
    label: 'What it can reach',
    tone: 'paper',
    items: [
      'Gmail',
      'GitHub',
      'Slack',
      'Linear',
      'Notion',
      'Google Calendar',
      'Jira',
      'Home Assistant',
      'Stripe',
      'Outlook',
      'Discord',
      'Nextcloud',
      'Zoom',
      'PostgreSQL and MySQL',
      'S3 buckets',
      'ntfy and Pushover',
      'Any MCP server',
    ],
  },
  {
    label: 'What you see',
    tone: 'sage',
    items: [
      'Live replies',
      'Every tool call',
      'The desktop, live',
      'Rooms',
      'Tasks and plans',
      'Home and an org chart',
      'Memories you can edit',
      'Dream reports',
      'Spend per agent',
      'An audit log',
      'A knowledge base',
      'Mermaid and math in chat',
    ],
  },
] as const
