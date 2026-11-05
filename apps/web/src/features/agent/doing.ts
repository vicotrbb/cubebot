type Doing = {
  activity: string
  pending_login?: boolean
  current_tool?: string | null
  current_target?: string | null
}

const IDLE: Record<string, string> = {
  dreaming: 'Tidying its memories',
  idle: 'Ready for work',
  sleeping: 'Asleep, wakes when asked',
  stopped: 'Stopped',
  retired: 'Retired',
}

const TOOLS: [RegExp, string, string?][] = [
  [/^(bash|terminal)$/, 'Running a command'],
  [/^read_file$/, 'Reading $', 'Reading a file'],
  [/^write_file$/, 'Writing $', 'Writing a file'],
  [/^edit_file$/, 'Editing $', 'Editing a file'],
  [/^(glob|grep)$/, 'Searching its files'],
  [/^browser_navigate$/, 'Browsing $', 'Browsing the web'],
  [/^browser_/, 'Using the browser'],
  [/^(computer|screenshot)$/, 'Using its desktop'],
  [/^vault_/, 'Signing in with the vault'],
  [/^request_login$/, 'Asking for a login'],
  [
    /^(search_memory|remember|forget|revise_memory)/,
    'Going through its memories',
  ],
  [/^load_skill$/, 'Loading a skill'],
  [/^files_put$/, 'Uploading $', 'Uploading a file'],
  [/^files_get$/, 'Downloading a file'],
  [/^files_share$/, 'Sharing a file'],
  [/^files_/, 'Looking through shared files'],
  [/^kb_(search|outline|read|fetch)$/, 'Reading the knowledge base'],
  [/^kb_/, 'Writing in the knowledge base'],
  [/^(delegate|send_message)$/, 'Working with other agents'],
  [/^broadcast$/, 'Broadcasting to every agent'],
  [/^create_routine|routine/, 'Setting up a routine'],
]

function short(target: string) {
  if (/^https?:\/\//.test(target)) {
    try {
      return new URL(target).hostname.replace(/^www\./, '')
    } catch {
      return target
    }
  }
  return target.split('/').filter(Boolean).at(-1) ?? target
}

function working(tool: string | null | undefined, target?: string | null) {
  if (!tool) return 'Thinking'
  const known = TOOLS.find(([pattern]) => pattern.test(tool))
  if (known) {
    const [, phrase, without] = known
    if (!phrase.includes('$')) return phrase
    return target ? phrase.replace('$', short(target)) : (without ?? phrase)
  }
  // An MCP tool is `<server>__<tool>`.
  const server = tool.split('__')
  if (server.length > 1) return `Using ${server[0]}`
  return `Using ${tool.replaceAll('_', ' ')}`
}

export function doingOf(agent: Doing) {
  if (agent.pending_login) return 'Waiting for you to log in'
  if (agent.activity === 'working')
    return working(agent.current_tool, agent.current_target)
  return IDLE[agent.activity] ?? agent.activity
}
