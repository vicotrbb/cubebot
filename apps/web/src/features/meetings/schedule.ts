import type { Schemas } from '../../api/types'
import {
  addDays,
  dayOf,
  instantOf,
  minutesOf,
  nowIn,
  timesOf,
} from '../calendar/model'

type Summary = Schemas['MeetingSummary']

export type When = { day: string; start: string; end: string }

const LENGTH = 30
const LAST_MINUTE = 1439

const pad = (value: number) => String(value).padStart(2, '0')
const clock = (minute: number) =>
  `${pad(Math.floor(minute / 60))}:${pad(minute % 60)}`

export function startingAt(day: string, minute: number): When {
  const next = minute >= 1440 ? addDays(day, 1) : day
  const start = minute % 1440
  return {
    day: next,
    start: clock(start),
    end: clock(Math.min(start + LENGTH, LAST_MINUTE)),
  }
}

export function nextHalfHour(zone: string, ms = Date.now()): When {
  const now = nowIn(zone, ms)
  return startingAt(now.day, Math.ceil(now.minute / LENGTH) * LENGTH)
}

export function whenOf(starts: string, ends: string, zone: string): When {
  const times = timesOf(
    {
      title: '',
      all_day: false,
      start: starts,
      end: ends,
      location: '',
      description: '',
    },
    zone,
  )
  return { day: times.startDay, start: times.startTime, end: times.endTime }
}

export function instantsOf(when: When, zone: string) {
  if (!/^\d{4}-\d{2}-\d{2}$/.test(when.day))
    throw new RangeError('Pick the day of the meeting.')
  const starts = instantOf(when.day, minutesOf(when.start), zone)
  const ends = instantOf(when.day, minutesOf(when.end), zone)
  if (Date.parse(ends) <= Date.parse(starts))
    throw new RangeError('The meeting ends before it starts.')
  return { starts_at: starts, ends_at: ends }
}

const LOCAL = /^[A-Za-z0-9._%+'-]{1,64}$/
const LABEL = /^[A-Za-z0-9](?:[A-Za-z0-9-]{0,61}[A-Za-z0-9])?$/

export function isAddress(raw: string) {
  const [local, domain, extra] = raw.trim().split('@')
  if (!local || !domain || extra !== undefined) return false
  const labels = domain.split('.')
  return (
    LOCAL.test(local) &&
    !local.startsWith('.') &&
    !local.endsWith('.') &&
    domain.length <= 253 &&
    labels.length > 1 &&
    labels.every((label) => LABEL.test(label))
  )
}

export function addressesIn(text: string) {
  return text
    .split(/[\s,;]+/)
    .map((part) => part.replace(/^<|>$/g, '').trim().toLowerCase())
    .filter(Boolean)
}

export function lengthOf(starts: string, ends: string) {
  return minutesWords((Date.parse(ends) - Date.parse(starts)) / 60_000)
}

export function minutesWords(span: number) {
  const minutes = Math.max(1, Math.round(span))
  if (minutes < 60) return `${minutes} min`
  const rest = minutes % 60
  return rest
    ? `${Math.floor(minutes / 60)} h ${rest} min`
    : `${minutes / 60} h`
}

export type Day = { day: string; meetings: Summary[] }

export function sectionsOf(meetings: Summary[], zone: string) {
  const live = meetings.filter((meeting) => meeting.state === 'live')
  const past = meetings.filter((meeting) => meeting.state === 'ended')
  const days = new Map<string, Summary[]>()
  for (const meeting of meetings) {
    if (meeting.state !== 'scheduled') continue
    const day = meeting.starts_at
      ? dayOf(Date.parse(meeting.starts_at), zone)
      : ''
    days.set(day, [...(days.get(day) ?? []), meeting])
  }
  const upcoming: Day[] = [...days]
    .sort(([a], [b]) => a.localeCompare(b))
    .map(([day, meetings]) => ({ day, meetings }))
  return { live, upcoming, past }
}

export function guestNameProblem(name: string) {
  const trimmed = name.normalize('NFC').trim()
  if (!trimmed) return 'Enter the name people will see.'
  if (!/^[\p{L}\p{N} .'-]+$/u.test(trimmed))
    return 'Use letters, numbers, spaces, periods, apostrophes or hyphens.'
  return ''
}
