export const repo = 'https://github.com/vicotrbb/cubebot'
export const sponsor = 'https://github.com/sponsors/vicotrbb'
export const install = 'curl -fsSL https://cubebot.io/install.sh | bash'
export const version = 'v0.2.2'
export const released = '2026-11-06'

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
    '27 native connectors and MCP servers',
    'Rooms, delegation, tasks and parallel lanes',
    'Memory, dreaming and a shared skill library',
    'Knowledge base, shared files and a vault',
    'Routines on schedules, webhooks and account events',
    'Voice calls, with speech that runs offline',
    'Teams, scoped grants, OpenID Connect, audit log',
    'Usage tracking and monthly budgets per agent',
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
    'Backup and restore from the console',
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
] as const

export const next = [
  {
    title: 'Agent teams',
    body: 'Agents grouped with an owner and shared work.',
  },
  { title: 'Video calls', body: 'Agents join your meetings.' },
  { title: 'Kanban', body: 'Tasks on a board you share with agents.' },
] as const

export const releases = [
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

// Brand marks for the wall's "What it can reach" tiles (keys of data/logos.ts).
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
  Outlook: 'microsoft-365',
  Discord: 'discord',
  Nextcloud: 'nextcloud',
  Zoom: 'zoom',
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
      'The Situation room',
      'Memories you can edit',
      'Dream reports',
      'Spend per agent',
      'An audit log',
      'A knowledge base',
      'Mermaid and math in chat',
    ],
  },
] as const
