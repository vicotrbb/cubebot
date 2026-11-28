import { isSelf, phrase, type Detail } from './audit'

/** RFC 4180: a field with a comma, quote or line break is quoted, its quotes doubled. */
export function csv(rows: unknown[][]) {
  return rows.map((row) => row.map(field).join(',')).join('\r\n') + '\r\n'
}

function field(value: unknown) {
  if (value === null || value === undefined) return ''
  const text = typeof value === 'object' ? JSON.stringify(value) : String(value)
  // A leading formula character would run as a formula in a spreadsheet.
  const safe = /^[=+\-@\t\r]/.test(text) ? `'${text}` : text
  return /[",\r\n]/.test(safe) ? `"${safe.replaceAll('"', '""')}"` : safe
}

type Entry = {
  sequence: number
  at: string
  kind: string
  actor?: string | null
  actor_name?: string | null
  agent?: string | null
  agent_name?: string | null
  resource?: string | null
  resource_name?: string | null
  detail: unknown
}

export const AUDIT_COLUMNS = [
  'sequence',
  'at',
  'actor',
  'actor_id',
  'agent',
  'agent_id',
  'event',
  'what',
  'resource',
  'resource_id',
  'detail',
]

export function auditCsv(entries: Entry[]) {
  return csv([
    AUDIT_COLUMNS,
    ...entries.map((entry) => {
      const detail = (entry.detail ?? {}) as Detail
      const self = isSelf(entry.kind, entry.actor, entry.resource)
      return [
        entry.sequence,
        entry.at,
        entry.actor_name,
        entry.actor,
        entry.agent_name,
        entry.agent,
        entry.kind,
        phrase(entry.kind, detail, self),
        entry.resource_name,
        entry.resource,
        Object.keys(detail).length ? detail : null,
      ]
    }),
  ])
}
