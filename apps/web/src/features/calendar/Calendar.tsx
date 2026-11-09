import { ChevronLeft, ChevronRight, Layers } from 'lucide-react'
import { useCallback, useEffect, useMemo, useState } from 'react'
import type { AgentSummary } from '../../api/types'
import { useAgents, useSession } from '../../app/queries'
import { Alert } from '../../components/primitives'
import { useProfile } from '../shell/AboutYou'
import { useIsPhone } from '../shell/viewport'
import { useCalendar } from './api'
import { addDays, nowIn, range, userZone, type View } from './model'
import { Agenda, MonthView } from './MonthView'
import { formatDay, type Item, type Look, type Source } from './parts'
import { Sources } from './Sources'
import { TimeGrid } from './TimeGrid'
import './calendar.css'

export type Place = { view: View; date?: string | undefined }

const VIEWS: [View, string, string][] = [
  ['day', 'Day', 'd'],
  ['week', 'Week', 'w'],
  ['month', 'Month', 'm'],
]
// Vendor colors are untrusted text; only a hex color reaches a style.
const HEX = /^#[0-9a-f]{3,8}$/i
const SPARE = [
  'var(--ocean)',
  'var(--rose-deep)',
  'var(--mint-deep)',
  'var(--lemon-deep)',
  'var(--plum)',
  'var(--coral)',
  'var(--sky-deep)',
]

/** Calendar events and agents' routines on one timeline, in the viewer's time zone. */
export function Calendar(props: Place & { onGo: (place: Place) => void }) {
  const profile = useProfile()
  if (profile.isPending) return <div className="page calendar" />
  return (
    <CalendarPage zone={userZone(profile.data?.time_zone ?? '')} {...props} />
  )
}

function CalendarPage({
  zone,
  view,
  date: wanted,
  onGo,
}: Place & { zone: string; onGo: (place: Place) => void }) {
  const session = useSession()
  const agents = useAgents()
  const phone = useIsPhone()
  const now = useNow(zone)
  const date = wanted ?? now.day
  const span = useMemo(
    () => range(view, date, zone, firstWeekday()),
    [view, date, zone],
  )
  const calendar = useCalendar(span.from, span.to, zone)
  const [hidden, toggle] = useHidden(session.data?.id ?? '')
  const [selected, setSelected] = useState<string | null>(null)
  const [sourcesOpen, setSourcesOpen] = useState(false)
  const sources = useMemo(() => calendar.data?.sources ?? [], [calendar.data])
  const paints = useMemo(
    () => paintsOf(sources, agents.data ?? []),
    [sources, agents.data],
  )
  const items = useMemo(
    () =>
      (calendar.data?.items ?? []).filter((item) => !hidden.has(item.source)),
    [calendar.data, hidden],
  )
  const go = useCallback(
    (next: Partial<Place>) => onGo({ view, date, ...next }),
    [onGo, view, date],
  )
  useKeys(view, date, now.day, go)
  const names = new Map(sources.map((source) => [source.id, source.name]))
  const look: Look = {
    zone,
    paint: (item: Item) => paints.get(item.source) ?? 'var(--ink-faint)',
    sourceName: (item: Item) => names.get(item.source) ?? '',
    selected,
    onSelect: (item) => setSelected(item.id),
  }
  const open = (day: string) => go({ view: 'day', date: day })
  return (
    <div className="page calendar">
      <header className="page__head calendar__head">
        <Heading view={view} date={date} days={span.days} />
        <div className="calendar__controls">
          <div className="tabs calendar__views" role="group" aria-label="View">
            {VIEWS.map(([id, label, key]) => (
              <button
                key={id}
                type="button"
                aria-pressed={id === view}
                aria-keyshortcuts={key}
                title={`${label} (${key.toUpperCase()})`}
                onClick={() => go({ view: id })}
              >
                {label}
              </button>
            ))}
          </div>
          <div className="calendar__step">
            <button
              type="button"
              className="btn btn--ghost btn--icon btn--sm"
              aria-label={`Previous ${view}`}
              onClick={() => go({ date: shift(view, date, -1) })}
            >
              <ChevronLeft />
            </button>
            <button
              type="button"
              className="btn btn--sm"
              aria-keyshortcuts="t"
              title="Today (T)"
              onClick={() => go({ date: now.day })}
            >
              Today
            </button>
            <button
              type="button"
              className="btn btn--ghost btn--icon btn--sm"
              aria-label={`Next ${view}`}
              onClick={() => go({ date: shift(view, date, 1) })}
            >
              <ChevronRight />
            </button>
          </div>
        </div>
        {phone && (
          <button
            type="button"
            className="btn btn--icon btn--sm calendar__toggle"
            aria-label="Calendars"
            title="Calendars"
            aria-expanded={sourcesOpen}
            aria-controls="cal-sources"
            onClick={() => setSourcesOpen(!sourcesOpen)}
          >
            <Layers />
          </button>
        )}
      </header>
      <div className="calendar__body">
        {(!phone || sourcesOpen) && (
          <aside
            id="cal-sources"
            className="calendar__sources"
            aria-label="Calendars and agents"
          >
            <Sources
              sources={sources}
              agents={agents.data ?? []}
              administrator={Boolean(session.data?.administrator)}
              hidden={hidden}
              onToggle={toggle}
              paint={(id) => paints.get(id) ?? 'var(--ink-faint)'}
            />
          </aside>
        )}
        <main
          className="calendar__main"
          aria-label="Calendar"
          aria-busy={calendar.isFetching}
        >
          {calendar.isError && (
            <Alert>The calendar could not be read. Retry shortly.</Alert>
          )}
          {view === 'month' ? (
            <>
              <MonthView
                days={span.days}
                month={date.slice(0, 7)}
                items={items}
                look={look}
                compact={phone}
                today={now.day}
                date={date}
                onPick={(day) => go({ date: day })}
                onOpen={open}
              />
              {phone && <Agenda day={date} items={items} look={look} />}
            </>
          ) : (
            <TimeGrid
              key={view}
              days={span.days}
              items={items}
              look={look}
              today={now.day}
              date={date}
              now={now.minute}
              onOpen={open}
            />
          )}
        </main>
      </div>
    </div>
  )
}

function Heading({
  view,
  date,
  days,
}: {
  view: View
  date: string
  days: string[]
}) {
  const [main, rest] = headingOf(view, date, days)
  return (
    <h1 aria-live="polite">
      {main} <span className="faint">{rest}</span>
    </h1>
  )
}

function headingOf(view: View, date: string, days: string[]): [string, string] {
  if (view === 'day')
    return [
      formatDay(date, { day: 'numeric', month: 'long', year: 'numeric' }),
      formatDay(date, { weekday: 'long' }),
    ]
  const first = view === 'month' ? date : (days[0] ?? date)
  const last = view === 'month' ? date : (days.at(-1) ?? date)
  if (first.slice(0, 7) === last.slice(0, 7))
    return [formatDay(first, { month: 'long' }), first.slice(0, 4)]
  const month = (day: string) => formatDay(day, { month: 'short' })
  const years =
    first.slice(0, 4) === last.slice(0, 4)
      ? first.slice(0, 4)
      : `${first.slice(0, 4)} – ${last.slice(0, 4)}`
  return [`${month(first)} – ${month(last)}`, years]
}

/** The day one view earlier or later; a month keeps the day number where it can. */
export function shift(view: View, date: string, step: 1 | -1) {
  if (view === 'day') return addDays(date, step)
  if (view === 'week') return addDays(date, 7 * step)
  const [year = 0, month = 1, day = 1] = date.split('-').map(Number)
  const target = new Date(Date.UTC(year, month - 1 + step, 1))
  const length = new Date(
    Date.UTC(target.getUTCFullYear(), target.getUTCMonth() + 1, 0),
  ).getUTCDate()
  return `${target.toISOString().slice(0, 8)}${String(Math.min(day, length)).padStart(2, '0')}`
}

/** The locale's first day of the week, where the calendar can start on it. */
function firstWeekday(): 0 | 1 {
  try {
    const locale = new Intl.Locale(navigator.language) as Intl.Locale & {
      getWeekInfo?: () => { firstDay: number }
      weekInfo?: { firstDay: number }
    }
    const first = locale.getWeekInfo?.().firstDay ?? locale.weekInfo?.firstDay
    return first === 7 ? 0 : 1
  } catch {
    return 1
  }
}

function useNow(zone: string) {
  const [now, setNow] = useState(() => nowIn(zone))
  useEffect(() => {
    const tick = () => setNow(nowIn(zone))
    tick()
    const timer = window.setInterval(tick, 30_000)
    return () => window.clearInterval(timer)
  }, [zone])
  return now
}

/** Hidden sources, remembered on this device for this person. */
function useHidden(person: string) {
  const key = `cubebot:${person}:calendar-hidden`
  const [hidden, setHidden] = useState(() => storedHidden(key))
  const toggle = (id: string) =>
    setHidden((current) => {
      const next = new Set(current)
      if (!next.delete(id)) next.add(id)
      try {
        localStorage.setItem(key, JSON.stringify([...next]))
      } catch {
        // Private windows only lose the convenience.
      }
      return next
    })
  return [hidden, toggle] as const
}

function storedHidden(key: string) {
  try {
    const stored: unknown = JSON.parse(localStorage.getItem(key) ?? '[]')
    return new Set(Array.isArray(stored) ? (stored as string[]) : [])
  } catch {
    return new Set<string>()
  }
}

function paintsOf(sources: Source[], agents: AgentSummary[]) {
  const paints = new Map<string, string>()
  let spare = 0
  for (const source of sources) {
    if (source.kind === 'agent') {
      const agent = agents.find((each) => `agent:${each.id}` === source.id)
      paints.set(source.id, agentPaint(agent))
    } else {
      const color = source.color && HEX.test(source.color) ? source.color : null
      paints.set(source.id, color ?? SPARE[spare++ % SPARE.length] ?? '')
    }
  }
  return paints
}

/** An agent's items carry its cube's paint, one shade deeper so they read on paper. */
function agentPaint(agent: AgentSummary | undefined) {
  const avatar = agent?.avatar
  if (!avatar) return 'var(--orange-deep)'
  if (avatar.color && HEX.test(avatar.color)) return avatar.color
  return avatar.paint === 'paper'
    ? 'var(--paper-base)'
    : `var(--${avatar.paint}-deep)`
}

const MOVES: Record<string, number> = {
  ArrowLeft: -1,
  ArrowRight: 1,
  ArrowUp: -7,
  ArrowDown: 7,
}
const KEYED_VIEWS: Record<string, View> = { d: 'day', w: 'week', m: 'month' }

function typing(target: EventTarget | null) {
  return (
    target instanceof HTMLElement &&
    (target.isContentEditable ||
      ['INPUT', 'TEXTAREA', 'SELECT'].includes(target.tagName))
  )
}

/** Arrows move the day (up and down a week in Month), T is today, D W M pick the view. */
function useKeys(
  view: View,
  date: string,
  today: string,
  go: (next: Partial<Place>) => void,
) {
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.metaKey || event.ctrlKey || event.altKey) return
      if (typing(event.target) || document.querySelector('dialog[open]')) return
      const move = MOVES[event.key]
      if (move && (view === 'month' || Math.abs(move) === 1)) {
        event.preventDefault()
        go({ date: addDays(date, move) })
        return
      }
      const key = event.key.toLowerCase()
      if (key === 't') go({ date: today })
      const picked = KEYED_VIEWS[key]
      if (picked) go({ view: picked })
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [view, date, today, go])
}
