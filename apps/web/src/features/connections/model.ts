import type { Schemas } from '../../api/types'

export type Catalog = Schemas['Catalog']
export type Manifest = Schemas['Manifest']
export type Connection = Schemas['ConnectionSummary']
export type Auth = Schemas['Auth']
export type Field = Schemas['Field']
export type Level = Schemas['Level']
export type Category = Schemas['Category']
export type Tool = Schemas['ToolSpec']

export const CATEGORIES: Record<Category, string> = {
  mail_calendar: 'Mail and calendar',
  files: 'Files',
  dev: 'Dev',
  work: 'Work',
  chat: 'Chat',
  home: 'Home',
  finance: 'Finance',
  notifications: 'Notifications',
  data: 'Data',
}

export const LEVELS: { id: Level; label: string; meaning: string }[] = [
  { id: 'read', label: 'Read', meaning: 'Search and read; change nothing.' },
  {
    id: 'write',
    label: 'Read and write',
    meaning: 'Also create and change things, but never send or post.',
  },
  {
    id: 'full',
    label: 'Full',
    meaning: 'Also send email and messages: anything that reaches people.',
  },
]

const RISKS = ['read', 'write', 'send'] as const
const REACH: Record<Level, number> = { read: 0, write: 1, full: 2 }

export function permitted(level: Level, risk: Tool['risk']) {
  return RISKS.indexOf(risk) <= REACH[level]
}

export const AUTH_LABEL: Record<Auth['kind'], string> = {
  api_key: 'Token',
  password: 'Password',
  oauth2: 'Sign in',
  github_app: 'GitHub App',
  linked: 'Another connection',
  client_credentials: 'App credentials',
  public: 'No sign-in',
}

export type Tone = 'none' | 'ok' | 'again' | 'error'

/** What a connector's card says about its accounts, worst news first. */
export function standing(
  connections: Connection[],
  ago: (at: string | null | undefined) => string,
): { tone: Tone; text: string } {
  if (connections.length === 0) return { tone: 'none', text: 'Not connected' }
  if (connections.some((c) => c.status === 'sign_in_again'))
    return { tone: 'again', text: 'Needs sign-in again' }
  const failing = connections.find((c) => c.status === 'error')
  if (failing)
    return {
      tone: 'error',
      text: failing.checked_at
        ? `Error · checked ${ago(failing.checked_at)}`
        : 'Error',
    }
  if (connections.length > 1)
    return { tone: 'ok', text: `${connections.length} accounts` }
  const [only] = connections
  return {
    tone: 'ok',
    text: only?.account ? `Connected as ${only.account}` : 'Connected',
  }
}

/** A connector matches every word of the search in its name, description, products or tools. */
export function matches(
  manifest: Manifest,
  query: string,
  category: Category | null,
) {
  if (category && manifest.category !== category) return false
  const words = query.toLowerCase().split(/\s+/).filter(Boolean)
  const haystack = [
    manifest.name,
    manifest.description,
    ...manifest.products.map((p) => p.name),
    ...manifest.tools.map((t) => t.name.replace(/_/g, ' ')),
  ]
    .join(' ')
    .toLowerCase()
  return words.every((word) => haystack.includes(word))
}

/** The categories that have a connector, in catalog order. */
export function categories(connectors: Manifest[]) {
  return (Object.keys(CATEGORIES) as Category[]).filter((category) =>
    connectors.some((c) => c.category === category),
  )
}

/** A free name for a new connection: the connector's id, numbered after the first. */
export function suggestedSlug(manifest: Manifest, taken: string[]) {
  const base = manifest.id.replace(/[^a-z0-9-]/g, '')
  if (!taken.includes(base)) return base
  for (let n = 2; ; n++)
    if (!taken.includes(`${base}-${n}`)) return `${base}-${n}`
}

export function slugProblem(slug: string) {
  return /^[a-z0-9][a-z0-9-]{0,31}$/.test(slug)
    ? ''
    : 'Use 1 to 32 lowercase letters, digits or hyphens, starting with a letter or digit.'
}

/** Guide text with this installation's redirect address filled in. */
export function filled(text: string, redirect: string) {
  return text.replaceAll('{redirect_uri}', redirect)
}

/** A typed value for a setting, or undefined to leave it out. */
export function value(field: Field, raw: string): unknown {
  const text = raw.trim()
  if (!text) return undefined
  if (field.kind.type === 'number') return Number(text)
  if (field.kind.type === 'lines')
    return text
      .split('\n')
      .map((line) => line.trim())
      .filter(Boolean)
  return text
}

/** The tools a connection offers, grouped by what they can do. */
export function grouped(manifest: Manifest, products: string[] | null) {
  const on = (tool: Tool) =>
    !tool.product || !products || products.includes(tool.product)
  return RISKS.map((risk) => ({
    risk,
    tools: manifest.tools.filter((tool) => tool.risk === risk && on(tool)),
  })).filter((group) => group.tools.length > 0)
}

export const RISK_LABEL: Record<Tool['risk'], string> = {
  read: 'Reads',
  write: 'Changes',
  send: 'Reaches people',
}
