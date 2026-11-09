import {
  CircleCheck,
  CircleDashed,
  CircleSlash,
  CircleX,
  Repeat,
} from 'lucide-react'
import type { MouseEvent } from 'react'
import type { Schemas } from '../../api/types'
import { cadence, formatRange, type Merged } from './model'

export type Item = Schemas['Item']
export type Source = Schemas['Source']
export type Point = { x: number; y: number }
export type Select = (item: Item, at: Point) => void

/** What every view needs to draw an item. */
export type Look = {
  zone: string
  paint: (item: Item) => string
  sourceName: (item: Item) => string
  selected: string | null
  onSelect: Select
}

const OUTCOMES: Record<string, [string, typeof CircleCheck]> = {
  complete: ['Done', CircleCheck],
  failed: ['Failed', CircleX],
  cancelled: ['Cancelled', CircleSlash],
}

function outcomeOf(item: Item) {
  const status = item.routine?.outcome ?? ''
  return { status, look: OUTCOMES[status] ?? ['Not finished', CircleDashed] }
}

/** The mark in front of a routine or a past run: what it is, and how a run ended. */
export function Glyph({ item }: { item: Item }) {
  if (item.kind === 'routine')
    return <Repeat className="cal-glyph" aria-hidden="true" />
  if (item.kind !== 'run') return null
  const { status, look } = outcomeOf(item)
  const Icon = look[1]
  return <Icon className="cal-glyph" data-outcome={status} aria-hidden="true" />
}

/** A digest reads "Inbox sweep · every 5 min"; its color and label say whose. */
export function titleOf(item: Item) {
  if (item.kind !== 'digest') return item.title
  return `${item.title} · ${cadence(item.routine?.schedule ?? '')}`
}

/** "288 runs a day", or a span when the days differ. */
function runsOf(item: Merged) {
  const counts = (item.days ?? [item]).map((day) => day.routine?.runs ?? 0)
  const least = Math.min(...counts)
  const most = Math.max(...counts)
  return least === most
    ? `${most} runs a day`
    : `${least} to ${most} runs a day`
}

export function labelOf(item: Merged, look: Look) {
  const when =
    item.kind === 'digest' ? runsOf(item) : formatRange(item, look.zone)
  const what: Record<string, string> = {
    routine: `routine of ${look.sourceName(item)}`,
    run: `${outcomeOf(item).look[0]}, run of ${look.sourceName(item)}`,
    digest: `routine of ${look.sourceName(item)}`,
    event: look.sourceName(item),
  }
  return `${titleOf(item)}, ${when}, ${what[item.kind] ?? ''}`
}

/** Where a popover should point: the click, or the item's edge for a keyboard press. */
export function pointOf(event: MouseEvent<HTMLElement>): Point {
  if (event.detail > 0) return { x: event.clientX, y: event.clientY }
  const box = event.currentTarget.getBoundingClientRect()
  return { x: box.right, y: box.top + Math.min(box.height / 2, 16) }
}

/** The props every item button shares. */
export function itemProps(item: Item, look: Look) {
  return {
    type: 'button' as const,
    'data-kind': item.kind,
    'data-selected': look.selected === item.id || undefined,
    'aria-label': labelOf(item, look),
    style: { '--c': look.paint(item) } as React.CSSProperties,
    onClick: (event: MouseEvent<HTMLElement>) => {
      event.stopPropagation()
      look.onSelect(item, pointOf(event))
    },
  }
}

const dayFormats = new Map<string, Intl.DateTimeFormat>()

/** A calendar day (`YYYY-MM-DD`) in words; days carry no zone, so they format as UTC. */
export function formatDay(day: string, options: Intl.DateTimeFormatOptions) {
  const key = JSON.stringify(options)
  let format = dayFormats.get(key)
  if (!format) {
    format = new Intl.DateTimeFormat(undefined, { ...options, timeZone: 'UTC' })
    dayFormats.set(key, format)
  }
  return format.format(new Date(`${day}T12:00:00Z`))
}

export const FULL_DAY: Intl.DateTimeFormatOptions = {
  weekday: 'long',
  day: 'numeric',
  month: 'long',
}
