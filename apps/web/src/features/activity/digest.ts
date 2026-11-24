import type { components } from '../../api/schema'

export type Item = components['schemas']['ActivityItem']

const NEEDS = [
  'watchdog',
  'attention',
  'approval_pending',
  'login_pending',
  'turn_waiting',
  'turn_failed',
]

export const needsYou = (item: Item) => NEEDS.includes(item.kind)

export function byUrgency(items: Item[]) {
  return [...items].sort(
    (a, b) =>
      NEEDS.indexOf(a.kind) - NEEDS.indexOf(b.kind) || b.at.localeCompare(a.at),
  )
}

export const keyOf = (item: Item) =>
  `${item.kind}:${item.turn ?? ''}:${item.agent ?? ''}:${item.meeting ?? ''}:${item.at}`

function plain(line: string) {
  return line
    .replace(/!\[[^\]]*\]\([^)]*\)/g, '')
    .replace(/\[([^\]]*)\]\([^)]*\)/g, '$1')
    .replace(/^\s{0,3}(#{1,6}\s+|>\s?|[-*+]\s+|\d+[.)]\s+)/, '')
    .replace(/\*\*|__|~~|`|(?<!\w)[*_](?=\S)|(?<=\S)[*_](?!\w)/g, '')
    .replace(/\s+/g, ' ')
    .trim()
}

export function excerpt(markdown: string, limit = 180) {
  const text = markdown
    .replace(/```[\s\S]*?(```|$)/g, '\n')
    .split('\n')
    .map(plain)
    .filter((line) => line && !/^[-|:\s]+$/.test(line))
    .join(' · ')
  if (text.length <= limit) return text
  const cut = text.slice(0, limit)
  const space = cut.lastIndexOf(' ')
  return `${(space > limit * 0.6 ? cut.slice(0, space) : cut).replace(/[\s·,;:]+$/, '')}…`
}
