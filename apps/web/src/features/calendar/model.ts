import type { Schemas } from '../../api/types'

type Item = Schemas['Item']
export type View = 'month' | 'week' | 'day'
export type Placed = {
  item: Item
  top: number
  height: number
  column: number
  columns: number
}

const MINUTE = 60_000
const DAY_MINUTES = 1440
// A routine run has no end; this keeps it tall enough to read and click.
export const POINT_MINUTES = 30
const WEEKDAYS = [
  'Sunday',
  'Monday',
  'Tuesday',
  'Wednesday',
  'Thursday',
  'Friday',
  'Saturday',
]

export function userZone(profileZone: string) {
  return profileZone || Intl.DateTimeFormat().resolvedOptions().timeZone
}

// Building a formatter costs far more than using one, and a month view asks thousands of times.
const formatters = new Map<string, Intl.DateTimeFormat>()

function formatter(
  zone: string,
  kind: string,
  options: Intl.DateTimeFormatOptions,
) {
  const key = `${kind}|${zone}`
  let found = formatters.get(key)
  if (!found) {
    found = new Intl.DateTimeFormat(kind === 'clock' ? 'en-US' : undefined, {
      ...options,
      timeZone: zone,
    })
    formatters.set(key, found)
  }
  return found
}

/** The zone's wall clock at an instant, read as if it were UTC. */
function wallClock(ms: number, zone: string) {
  const parts = formatter(zone, 'clock', {
    hourCycle: 'h23',
    year: 'numeric',
    month: 'numeric',
    day: 'numeric',
    hour: 'numeric',
    minute: 'numeric',
    second: 'numeric',
  }).formatToParts(ms)
  const n = (type: string) => Number(parts.find((p) => p.type === type)?.value)
  return Date.UTC(
    n('year'),
    n('month') - 1,
    n('day'),
    n('hour'),
    n('minute'),
    n('second'),
  )
}

function dayOf(ms: number, zone: string) {
  return new Date(wallClock(ms, zone)).toISOString().slice(0, 10)
}

function addDays(day: string, days: number) {
  const date = new Date(`${day}T00:00:00Z`)
  date.setUTCDate(date.getUTCDate() + days)
  return date.toISOString().slice(0, 10)
}

/** The instant a local day starts, as milliseconds. */
function midnight(day: string, zone: string) {
  const wanted = Date.parse(`${day}T00:00:00Z`)
  let guess = wanted
  // Two passes settle the offset even when it differs at the guess and at midnight.
  for (let i = 0; i < 2; i++) guess -= wallClock(guess, zone) - wanted
  // A clock that jumps over 00:00 has no midnight; the day starts at the first instant after the jump.
  while (dayOf(guess, zone) < day) guess += 15 * MINUTE
  return guess
}

function weekStartOf(day: string, weekStart: 0 | 1) {
  const weekday = new Date(`${day}T00:00:00Z`).getUTCDay()
  return addDays(day, -((weekday - weekStart + 7) % 7))
}

function span(first: string, last: string) {
  const days: string[] = []
  for (let day = first; day <= last; day = addDays(day, 1)) days.push(day)
  return days
}

export function range(
  view: View,
  anchor: string,
  zone: string,
  weekStart: 0 | 1,
) {
  const month = anchor.slice(0, 8)
  const monthEnd = addDays(addDays(`${month}01`, 32).slice(0, 8) + '01', -1)
  const week = weekStartOf(anchor, weekStart)
  const bounds: Record<View, [string, string]> = {
    month: [
      weekStartOf(`${month}01`, weekStart),
      addDays(weekStartOf(monthEnd, weekStart), 6),
    ],
    week: [week, addDays(week, 6)],
    day: [anchor, anchor],
  }
  const [first, last] = bounds[view]
  return {
    from: new Date(midnight(first, zone)).toISOString(),
    to: new Date(midnight(addDays(last, 1), zone)).toISOString(),
    days: span(first, last),
  }
}

function endOf(item: Item) {
  return item.end
    ? Date.parse(item.end)
    : Date.parse(item.start) + POINT_MINUTES * MINUTE
}

export function localDay(item: Item, zone: string) {
  if (item.all_day) {
    const end = item.end ?? addDays(item.start, 1)
    return span(item.start, addDays(end, -1))
  }
  const start = Date.parse(item.start)
  // The end instant itself belongs to the next day, so step back from it.
  const last =
    item.end && Date.parse(item.end) > start ? Date.parse(item.end) - 1 : start
  return span(dayOf(start, zone), dayOf(last, zone))
}

// Minutes on the grid are wall-clock, so a clock change shifts items the way the hour labels do.
function place(
  item: Item,
  day: string,
  zone: string,
): Omit<Placed, 'column' | 'columns'> {
  const base = Date.parse(`${day}T00:00:00Z`)
  const minutes = (ms: number) => (wallClock(ms, zone) - base) / MINUTE
  const top = Math.max(0, minutes(Date.parse(item.start)))
  const bottom = Math.min(DAY_MINUTES, minutes(endOf(item)))
  const height = Math.max(bottom - top, POINT_MINUTES)
  return { item, top: Math.min(top, DAY_MINUTES - height), height }
}

/** Overlapping items form a cluster; each takes the first free column in it. */
export function layoutDay(items: Item[], day: string, zone: string): Placed[] {
  const timed = items
    .filter((item) => !item.all_day && localDay(item, zone).includes(day))
    .map((item) => place(item, day, zone))
    .sort((a, b) => a.top - b.top || b.height - a.height)
  const placed: Placed[] = []
  let cluster: Placed[] = []
  let clusterEnd = 0
  const close = () => {
    const columns = Math.max(...cluster.map((p) => p.column + 1))
    for (const p of cluster) p.columns = columns
    cluster = []
  }
  for (const base of timed) {
    if (cluster.length && base.top >= clusterEnd) close()
    const taken = new Set(
      cluster.filter((p) => p.top + p.height > base.top).map((p) => p.column),
    )
    let column = 0
    while (taken.has(column)) column++
    const next = { ...base, column, columns: 1 }
    cluster.push(next)
    placed.push(next)
    clusterEnd =
      cluster.length === 1
        ? base.top + base.height
        : Math.max(clusterEnd, base.top + base.height)
  }
  if (cluster.length) close()
  return placed
}

export function formatTime(instant: string, zone: string) {
  return formatter(zone, 'time', {
    hour: '2-digit',
    minute: '2-digit',
    hourCycle: 'h23',
  }).format(new Date(instant))
}

export function formatRange(item: Item, zone: string) {
  if (item.all_day) return 'All day'
  const start = formatTime(item.start, zone)
  return item.end ? `${start} – ${formatTime(item.end, zone)}` : start
}

const clock = (hour: string, minute: string) =>
  `${hour.padStart(2, '0')}:${minute.padStart(2, '0')}`

const NUMBER = /^\d+$/

function dayPhrase(weekday: string) {
  if (weekday === '*') return 'Every day'
  if (weekday === '1-5') return 'Every weekday'
  const name = NUMBER.test(weekday) ? WEEKDAYS[Number(weekday)] : undefined
  return name && `Every ${name}`
}

/** A sentence for the common schedules; `null` means show the cron itself. */
export function cronPhrase(schedule: string) {
  const [minute = '', hour = '', date, month, weekday = '', ...rest] = schedule
    .trim()
    .split(/\s+/)
  if (rest.length || date !== '*' || month !== '*' || !NUMBER.test(minute))
    return null
  if (hour === '*')
    return minute === '0' && weekday === '*' ? 'Every hour' : null
  if (Number(hour) > 23 || Number(minute) > 59) return null
  const day = NUMBER.test(hour) ? dayPhrase(weekday) : null
  return day ? `${day} at ${clock(hour, minute)}` : null
}
