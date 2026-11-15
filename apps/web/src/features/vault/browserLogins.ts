import { TYPABLE } from './ItemForm'

export type Login = {
  name: string
  site: string
  username: string
  secret: string
}

export type Skip =
  'not_web' | 'no_username' | 'no_password' | 'untypable' | 'too_long'

export const SKIP_LABEL: Record<Skip, [string, string]> = {
  not_web: ['is not a website', 'are not websites'],
  no_username: ['has no username', 'have no username'],
  no_password: ['has no password', 'have no password'],
  untypable: [
    'holds characters the desktop cannot type',
    'hold characters the desktop cannot type',
  ],
  too_long: [
    'is longer than the vault holds',
    'are longer than the vault holds',
  ],
}

const COLUMNS = {
  name: ['name', 'title'],
  url: ['url', 'login_uri', 'origin'],
  username: ['username', 'login_username'],
  password: ['password', 'login_password'],
} as const

export function csvRows(text: string): string[][] {
  const rows: string[][] = []
  let row: string[] = []
  let cell = ''
  let quoted = false
  for (let i = 0; i < text.length; i++) {
    const c = text[i]
    if (quoted && c === '"' && text[i + 1] === '"') cell += text[i++]
    else if (c === '"') quoted = !quoted
    else if (quoted || (c !== ',' && c !== '\n' && c !== '\r')) cell += c
    else if (c === ',') {
      row.push(cell)
      cell = ''
    } else if (c === '\n' || text[i + 1] !== '\n') {
      rows.push([...row, cell])
      row = []
      cell = ''
    }
  }
  if (cell || row.length) rows.push([...row, cell])
  return rows.filter((r) => r.some(Boolean))
}

function origin(url: string) {
  try {
    const parsed = new URL(url)
    return /^https?:$/.test(parsed.protocol) && parsed.hostname
      ? parsed.origin
      : ''
  } catch {
    return ''
  }
}

function login(name: string, url: string, username: string, secret: string) {
  const site = origin(url.trim())
  if (!site) return 'not_web'
  if (!username) return 'no_username'
  if (!secret) return 'no_password'
  if (!TYPABLE.test(username) || !TYPABLE.test(secret)) return 'untypable'
  const host = new URL(site).hostname.replace(/^www\./, '')
  return { name: name.trim() || host, site, username, secret }
}

const fits = (login: Login) =>
  login.name.length <= 255 &&
  login.username.length <= 255 &&
  login.site.length <= 255 &&
  login.secret.length <= 4096

export function browserLogins(text: string) {
  const [head = [], ...body] = csvRows(text.replace(/^\uFEFF/, ''))
  const header = head.map((cell) => cell.trim().toLowerCase())
  const at = (names: readonly string[]) =>
    header.findIndex((cell) => names.includes(cell))
  const [name, url, username, password] = [
    at(COLUMNS.name),
    at(COLUMNS.url),
    at(COLUMNS.username),
    at(COLUMNS.password),
  ]
  if (url < 0 || username < 0 || password < 0) return null
  const logins: Login[] = []
  const skipped: Partial<Record<Skip, number>> = {}
  for (const row of body) {
    const found = login(
      row[name] ?? '',
      row[url] ?? '',
      row[username] ?? '',
      row[password] ?? '',
    )
    if (typeof found !== 'string' && fits(found)) {
      logins.push(found)
      continue
    }
    const reason = typeof found === 'string' ? found : 'too_long'
    skipped[reason] = (skipped[reason] ?? 0) + 1
  }
  return { logins, skipped }
}
