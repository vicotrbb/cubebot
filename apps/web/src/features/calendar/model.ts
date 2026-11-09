import type { Schemas } from '../../api/types'

type Item = Schemas['Item']
export type View = 'month' | 'week' | 'day'
export type Placed = {
  item: Item
  top: number
  height: number
  column: number
  columns: number
  /** How many earlier items this one is stacked over, when its cluster cascades. */
  indent: number
}

const MINUTE = 60_000
const DAY_MINUTES = 1440
// A routine run has no end; this keeps it tall enough to read and click.
export const POINT_MINUTES = 30
// Items starting this many minutes apart leave the earlier title showing, so they stack instead of splitting.
const CASCADE = 25
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

export function dayOf(ms: number, zone: string) {
  return new Date(wallClock(ms, zone)).toISOString().slice(0, 10)
}

export function addDays(day: string, days: number) {
  const date = new Date(`${day}T00:00:00Z`)
  date.setUTCDate(date.getUTCDate() + days)
  return date.toISOString().slice(0, 10)
}

/** The instant a wall-clock minute of a local day falls on, as milliseconds. */
function wallInstant(day: string, minute: number, zone: string) {
  const wanted = Date.parse(`${day}T00:00:00Z`) + minute * MINUTE
  let guess = wanted
  // Two passes settle the offset even when it differs at the guess and at the wanted time.
  for (let i = 0; i < 2; i++) guess -= wallClock(guess, zone) - wanted
  return guess
}

export function instantOf(day: string, minute: number, zone: string) {
  return new Date(wallInstant(day, minute, zone)).toISOString()
}

/** The instant a local day starts, as milliseconds. */
function midnight(day: string, zone: string) {
  let guess = wallInstant(day, 0, zone)
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
): Omit<Placed, 'column' | 'columns' | 'indent'> {
  const base = Date.parse(`${day}T00:00:00Z`)
  const minutes = (ms: number) => (wallClock(ms, zone) - base) / MINUTE
  const top = Math.max(0, minutes(Date.parse(item.start)))
  const bottom = Math.min(DAY_MINUTES, minutes(endOf(item)))
  const height = Math.max(bottom - top, POINT_MINUTES)
  return { item, top: Math.min(top, DAY_MINUTES - height), height }
}

const overlap = (a: Placed, b: Placed) =>
  a.top < b.top + b.height && b.top < a.top + a.height

/** Whether every overlapping pair in a cluster starts far enough apart to stack. */
function cascades(cluster: Placed[]) {
  return cluster.every((a, at) =>
    cluster
      .slice(at + 1)
      .every((b) => !overlap(a, b) || b.top - a.top >= CASCADE),
  )
}

/**
 * Overlapping items form a cluster. When their starts are far apart each later one is stacked
 * over the earlier ones, indented; otherwise each takes the first free column.
 */
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
    const stacked = cascades(cluster)
    cluster.forEach((p, at) => {
      p.columns = stacked ? 1 : columns
      if (!stacked) return
      p.column = 0
      p.indent = cluster.slice(0, at).filter((q) => overlap(p, q)).length
    })
    cluster = []
  }
  for (const base of timed) {
    if (cluster.length && base.top >= clusterEnd) close()
    const taken = new Set(
      cluster.filter((p) => p.top + p.height > base.top).map((p) => p.column),
    )
    let column = 0
    while (taken.has(column)) column++
    const next = { ...base, column, columns: 1, indent: 0 }
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

/** Today in the zone, and how many wall-clock minutes of it have passed. */
export function nowIn(zone: string, ms = Date.now()) {
  const day = dayOf(ms, zone)
  return {
    day,
    minute: (wallClock(ms, zone) - Date.parse(`${day}T00:00:00Z`)) / MINUTE,
  }
}

export type Bar = { item: Item; first: number; last: number; lane: number }

// Bars over whole days (events and digests) claim the top lanes, longest first; then the day's timed items.
const rank = (item: Item, first: number, last: number) =>
  item.all_day || last > first ? 0 : 2
const digestLast = (a: Item, b: Item) =>
  Number(a.kind === 'digest') - Number(b.kind === 'digest')

/**
 * A week row of the month: each item a bar over its days in the first lane free across them.
 * When `lanes` cannot hold every bar, the last lane is kept for each day's "more" count.
 */
export function weekLanes(
  items: Item[],
  days: string[],
  zone: string,
  lanes: number,
) {
  const spans = items.flatMap((item) => {
    const covered = localDay(item, zone)
      .map((day) => days.indexOf(day))
      .filter((at) => at >= 0)
    if (!covered.length) return []
    const first = Math.min(...covered)
    const last = Math.max(...covered)
    return [{ item, first, last, order: rank(item, first, last) }]
  })
  spans.sort(
    (a, b) =>
      a.order - b.order ||
      a.first - b.first ||
      b.last - a.last ||
      digestLast(a.item, b.item),
  )
  const taken: boolean[][] = []
  // Timed items stack under the bars over their day, as the all-day band sits over the hours.
  const barred = days.map(() => 0)
  const bars: Bar[] = spans.map(({ item, first, last, order }) => {
    let lane = order === 2 ? Math.max(...barred.slice(first, last + 1)) : 0
    const free = (row: boolean[] = []) =>
      row.slice(first, last + 1).every((cell) => !cell)
    while (!free(taken[lane])) lane++
    const row = (taken[lane] ??= [])
    for (let at = first; at <= last; at++) {
      row[at] = true
      if (order < 2) barred[at] = Math.max(barred[at] ?? 0, lane + 1)
    }
    return { item, first, last, lane }
  })
  const shown = bars.some((bar) => bar.lane >= lanes) ? lanes - 1 : lanes
  const more = days.map(
    (_, at) =>
      bars.filter(
        (bar) => bar.lane >= shown && bar.first <= at && at <= bar.last,
      ).length,
  )
  return { bars: bars.filter((bar) => bar.lane < shown), more }
}

/** What makes a day busy: its bars, except digests, which mark every day a routine runs all day. */
export function busyOn(bars: Bar[], column: number) {
  return bars.filter(
    (bar) =>
      bar.item.kind !== 'digest' && bar.first <= column && column <= bar.last,
  )
}

/** How often a digest's routine runs, in words. */
export function cadence(schedule: string) {
  const [minute = '', hour] = schedule.trim().split(/\s+/)
  const every = /^\*\/(\d+)$/.exec(minute)?.[1]
  if (hour === '*' && every) return `every ${every} min`
  if (hour === '*' && minute === '*') return 'every minute'
  return 'many times a day'
}

/** One routine's digests on consecutive days, as one item over those days; `days` keeps each. */
export type Merged = Item & { days?: Item[] }

export function mergeDigests(items: Item[]): Merged[] {
  // The server sends items in order of start; a day out of order would only start a new bar.
  const merged: Merged[] = []
  const open = new Map<string, Merged>()
  for (const item of items) {
    const routine = item.kind === 'digest' ? item.routine?.routine : undefined
    const last = routine ? open.get(routine) : undefined
    if (last && last.end === item.start) {
      last.end = item.end ?? null
      last.days?.push(item)
      continue
    }
    const next = routine ? { ...item, days: [item] } : item
    if (routine) open.set(routine, next)
    merged.push(next)
  }
  return merged
}

/** The day one view earlier or later; a month keeps the day number where it can. */
export function shift(view: View, date: string, step: number) {
  if (view === 'day') return addDays(date, step)
  if (view === 'week') return addDays(date, 7 * step)
  const [year = 0, month = 1, day = 1] = date.split('-').map(Number)
  const target = new Date(Date.UTC(year, month - 1 + step, 1))
  const length = new Date(
    Date.UTC(target.getUTCFullYear(), target.getUTCMonth() + 1, 0),
  ).getUTCDate()
  return `${target.toISOString().slice(0, 8)}${String(Math.min(day, length)).padStart(2, '0')}`
}

/** Days from `first` on, as `range` gives them: the phone's three-day week. */
export function stretch(first: string, count: number, zone: string) {
  const last = addDays(first, count - 1)
  return {
    from: range('day', first, zone, 1).from,
    to: range('day', last, zone, 1).to,
    days: span(first, last),
  }
}

const DAY_PATTERN = /^\d{4}-\d{2}-\d{2}$/

/** A real calendar day as `YYYY-MM-DD`; the pattern alone lets through 2026-02-31. */
export function isDay(value: unknown): value is string {
  return (
    typeof value === 'string' &&
    DAY_PATTERN.test(value) &&
    !Number.isNaN(Date.parse(value)) &&
    new Date(value).toISOString().startsWith(value)
  )
}

/** A wall-clock minute of a day, or `null` for an all-day slot. */
export type Slot = { day: string; minute: number | null }

// A routine made from an all-day slot starts the working morning.
const MORNING = 9 * 60

/** A weekly schedule on the slot's weekday and time: Tuesday 14:30 is `30 14 * * 2`. */
export function cronOf({ day, minute }: Slot) {
  const at = minute ?? MORNING
  const weekday = new Date(`${day}T12:00:00Z`).getUTCDay()
  return `${at % 60} ${Math.floor(at / 60)} * * ${weekday}`
}

/** An event's editable fields; times as an item has them, all-day ends exclusive. */
export type Draft = {
  title: string
  all_day: boolean
  start: string
  end: string
  location: string
  description: string
}

/** A new event over the slot: an hour from its minute, or its whole day. */
export function newDraft(slot: Slot, zone: string): Draft {
  const blank = { title: '', location: '', description: '' }
  if (slot.minute === null)
    return {
      ...blank,
      all_day: true,
      start: slot.day,
      end: addDays(slot.day, 1),
    }
  return {
    ...blank,
    all_day: false,
    start: instantOf(slot.day, slot.minute, zone),
    end: instantOf(slot.day, slot.minute + 60, zone),
  }
}

export function draftOf(item: Item): Draft {
  return {
    title: item.title,
    all_day: item.all_day,
    start: item.start,
    end: item.end ?? item.start,
    location: item.event?.location ?? '',
    description: item.event?.description ?? '',
  }
}

/**
 * Only what the form changed. A description arrives cut short or absent, so it goes only when
 * edited; times go together, with the all-day flag when that changed.
 */
export function changes(before: Draft, after: Draft) {
  const changed: Partial<Draft> = {}
  for (const key of ['title', 'location', 'description'] as const)
    if (after[key] !== before[key]) changed[key] = after[key]
  if (after.all_day !== before.all_day) changed.all_day = after.all_day
  const moved = (key: 'start' | 'end') =>
    Date.parse(after[key]) !== Date.parse(before[key])
  if (changed.all_day !== undefined || moved('start') || moved('end')) {
    changed.start = after.start
    changed.end = after.end
  }
  return changed
}

/** One day of a merged digest; a day it does not cover answers with its first. */
export function digestDay(item: Merged, day: string): Item {
  return item.days?.find((each) => each.start === day) ?? item
}

/** The form's start and end: days, and `HH:MM` wall-clock times in the zone. */
export type Times = {
  startDay: string
  startTime: string
  endDay: string
  endTime: string
}

const HH_MM = /^([01]\d|2[0-3]):([0-5]\d)$/

/** A time input's `HH:MM` as minutes; anything else is refused rather than misread. */
export function minutesOf(time: string) {
  const [, hour, minute] = HH_MM.exec(time) ?? []
  if (!hour || !minute) throw new RangeError('Write times as HH:MM.')
  return Number(hour) * 60 + Number(minute)
}

export function timesOf(draft: Draft, zone: string): Times {
  if (draft.all_day)
    return {
      startDay: draft.start,
      startTime: '09:00',
      endDay: addDays(draft.end, -1),
      endTime: '10:00',
    }
  // A time input takes ASCII HH:MM only, never the locale's way of writing a time.
  const at = (instant: string) => {
    const wall = new Date(wallClock(Date.parse(instant), zone)).toISOString()
    return { day: wall.slice(0, 10), time: wall.slice(11, 16) }
  }
  const start = at(draft.start)
  const end = at(draft.end)
  return {
    startDay: start.day,
    startTime: start.time,
    endDay: end.day,
    endTime: end.time,
  }
}

/** The draft with the form's times; an all-day end is the day after the last. */
export function withTimes(draft: Draft, times: Times, zone: string): Draft {
  if (!isDay(times.startDay) || !isDay(times.endDay))
    throw new RangeError('Pick the days it starts and ends.')
  if (draft.all_day)
    return { ...draft, start: times.startDay, end: addDays(times.endDay, 1) }
  return {
    ...draft,
    start: instantOf(times.startDay, minutesOf(times.startTime), zone),
    end: instantOf(times.endDay, minutesOf(times.endTime), zone),
  }
}
