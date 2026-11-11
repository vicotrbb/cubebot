import { usePrefetchQuery, useQueryClient } from '@tanstack/react-query'
import { ChevronLeft, ChevronRight, Layers, Plus } from 'lucide-react'
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import {
  useAgents,
  usePreferences,
  useSavePreferences,
  useSession,
} from '../../app/queries'
import type { AgentSummary } from '../../api/types'
import { Skeleton } from '../../components/primitives'
import { useProfile } from '../shell/AboutYou'
import { useIsPhone } from '../shell/viewport'
import { calendarQuery, useCalendar } from './api'
import { Editor, type Editing } from './EventForm'
import {
  addDays,
  mergeDigests,
  nowIn,
  range,
  shift,
  stretch,
  userZone,
  type Slot,
  type View,
} from './model'
import { Agenda, MonthView } from './MonthView'
import {
  formatDay,
  type Item,
  type Look,
  type Point,
  type Source,
} from './parts'
import { Popover, type Selection } from './Popover'
import { NoCalendars, Sources } from './Sources'
import { TimeGrid } from './TimeGrid'
import './calendar.css'

export type Place = { view: View; date?: string | undefined }

const VIEWS: [View, string, string][] = [
  ['day', 'Day', 'd'],
  ['week', 'Week', 'w'],
  ['month', 'Month', 'm'],
]
// A phone's week is three days at a time, paged like iOS Calendar's multi-day view.
const PHONE_WEEK = 3
const SOURCES_PANE = 'calendar-sources'
// Vendor colors are untrusted text; only a hex color reaches a style.
const HEX = /^#(?:[0-9a-f]{3,4}|[0-9a-f]{6}|[0-9a-f]{8})$/i
// Never the orange accent: it marks today, now and selection.
const SPARE = [
  'var(--ocean)',
  'var(--rose-deep)',
  'var(--lemon-deep)',
  'var(--sky-deep)',
  'var(--mint-deep)',
  'var(--sage-deep)',
  'var(--plum)',
]

/** Calendar events and agents' routines on one timeline, in the viewer's time zone. */
export function Calendar(props: Place & { onGo: (place: Place) => void }) {
  const profile = useProfile()
  if (profile.isPending)
    return (
      <div className="page calendar">
        <Warm {...props} />
      </div>
    )
  return (
    <CalendarPage zone={userZone(profile.data?.time_zone ?? '')} {...props} />
  )
}

function Warm({ view, date }: Place) {
  const phone = useIsPhone()
  const zone = userZone('')
  const span = spanOf(view, date ?? nowIn(zone).day, zone, phone)
  usePrefetchQuery(calendarQuery(span.from, span.to, zone))
  return null
}

function useNeighbors(
  calendar: { isSuccess: boolean; isPlaceholderData: boolean },
  view: View,
  date: string,
  zone: string,
  phone: boolean,
) {
  const client = useQueryClient()
  const settled = calendar.isSuccess && !calendar.isPlaceholderData
  useEffect(() => {
    if (!settled) return
    for (const by of [-1, 1] as const) {
      const next = spanOf(view, step(view, date, by, phone), zone, phone)
      void client.prefetchQuery(calendarQuery(next.from, next.to, zone))
    }
  }, [settled, client, view, date, zone, phone])
}

/** The days a view shows and the instants they span. */
function spanOf(view: View, date: string, zone: string, phone: boolean) {
  if (phone && view === 'week') return stretch(date, PHONE_WEEK, zone)
  return range(view, date, zone, firstWeekday())
}

function step(view: View, date: string, by: 1 | -1, phone: boolean) {
  return phone && view === 'week'
    ? addDays(date, PHONE_WEEK * by)
    : shift(view, date, by)
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
    () => spanOf(view, date, zone, phone),
    [view, date, zone, phone],
  )
  const calendar = useCalendar(span.from, span.to, zone)
  useNeighbors(calendar, view, date, zone, phone)
  // The app shell renders pages only once the session is known, so the key is final here.
  const [hidden, toggle] = useHidden(session.data?.id ?? '')
  const overlays = useOverlays(date)
  const [announced, setAnnounced] = useState('')
  const pane = useSourcesPane(phone)
  const sources = useMemo(() => calendar.data?.sources ?? [], [calendar.data])
  const paints = useMemo(() => paintsOf(sources), [sources])
  const items = useMemo(
    () =>
      mergeDigests(
        (calendar.data?.items ?? []).filter((item) => !hidden.has(item.source)),
      ),
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
    selected: overlays.selected,
    onSelect: overlays.select,
  }
  const create = overlays.create
  const heading = headingOf(view, date, span.days, phone)
  // Announce a new period once, when a step button changes it.
  const travel = (to: string) => {
    const next = headingOf(view, to, spanOf(view, to, zone, phone).days, phone)
    if (next.join(' ') !== heading.join(' ')) setAnnounced(next.join(' '))
    go({ date: to })
  }
  const open = (day: string) => go({ view: 'day', date: day })
  const noCalendars =
    calendar.isSuccess && !sources.some((source) => source.kind === 'calendar')
  return (
    <div className="page calendar">
      <Head
        view={view}
        heading={heading}
        announced={announced}
        pane={pane}
        onView={(id) => go({ view: id })}
        onStep={(by) => travel(by ? step(view, date, by, phone) : now.day)}
        onNew={() => create(newSlot(date, now))}
      />
      <div className="calendar__body" data-sources={pane.shown || undefined}>
        {pane.shown && (
          <aside
            id="cal-sources"
            className="calendar__sources"
            aria-label="Calendars and agents"
            aria-busy={calendar.isPending}
          >
            {calendar.data ? (
              <Sources
                sources={sources}
                agents={agents.data ?? []}
                administrator={Boolean(session.data?.administrator)}
                hidden={hidden}
                onToggle={toggle}
                paint={(id) => paints.get(id) ?? 'var(--ink-faint)'}
              />
            ) : (
              !calendar.isError && <Skeleton lines={4} width="80%" />
            )}
          </aside>
        )}
        <main
          className="calendar__main"
          aria-label="Calendar"
          aria-busy={calendar.isFetching}
        >
          <Notices
            calendar={calendar}
            noCalendars={noCalendars && !pane.shown}
            administrator={Boolean(session.data?.administrator)}
          />
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
                onCreate={create}
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
              onCreate={create}
            />
          )}
        </main>
      </div>
      <Overlays
        overlays={overlays}
        look={look}
        phone={phone}
        sources={sources}
        agents={agents.data}
      />
    </div>
  )
}

/** The open popover or editor, and how an item or a slot opens one. */
function useOverlays(date: string) {
  const [selection, setSelection] = useState<Selection | null>(null)
  const [editing, setEditing] = useState<Editing | null>(null)
  // The editor unmounts rather than closes, so it returns focus itself: to the item or the button.
  const returnTo = useRef<Element | null>(null)
  const open = (next: Editing) => {
    returnTo.current = document.activeElement
    setSelection(null)
    setEditing(next)
  }
  return {
    selection,
    editing,
    selected: selection?.item.id ?? null,
    select: (item: Item, at: Point) =>
      setSelection({
        item,
        at,
        day: dayAt(at) ?? date,
        opener: document.activeElement as HTMLElement | null,
      }),
    create: (slot: Slot) => open({ slot }),
    edit: (item: Item) => open({ item }),
    close: () => setSelection(null),
    closeEditor: () => {
      setEditing(null)
      const back = returnTo.current
      // While the modal editor is still up everything else is inert, so focus waits a frame.
      requestAnimationFrame(() => {
        if (back instanceof HTMLElement && back.isConnected) back.focus()
      })
    },
  }
}

function Overlays({
  overlays,
  look,
  phone,
  sources,
  agents = [],
}: {
  overlays: ReturnType<typeof useOverlays>
  look: Look
  phone: boolean
  sources: Source[]
  agents: AgentSummary[] | undefined
}) {
  const { selection, editing } = overlays
  return (
    <>
      {selection && (
        <Popover
          key={`${selection.item.id}:${selection.at.x}:${selection.at.y}`}
          selection={selection}
          sheet={phone}
          look={look}
          onClose={overlays.close}
          onEdit={overlays.edit}
        />
      )}
      {editing && (
        <Editor
          editing={editing}
          sources={sources}
          agents={agents}
          zone={look.zone}
          onClose={overlays.closeEditor}
        />
      )}
    </>
  )
}

/** The day under a point: a month's cell, or the time grid's column at that x. */
function dayAt({ x, y }: Point) {
  for (const cell of document.querySelectorAll<HTMLElement>('[data-day]')) {
    const box = cell.getBoundingClientRect()
    const column = cell.classList.contains('timegrid__col')
    if (
      x >= box.left &&
      x < box.right &&
      (column || (y >= box.top && y < box.bottom))
    )
      return cell.dataset.day
  }
  return undefined
}

/** "New" starts at the next hour today, or the working morning on another day. */
function newSlot(date: string, now: { day: string; minute: number }): Slot {
  if (date !== now.day) return { day: date, minute: 9 * 60 }
  return {
    day: date,
    minute: Math.min(23 * 60, Math.ceil(now.minute / 60) * 60),
  }
}

/** The loading sweep, a failed read with Retry, and the missing-calendars note while the pane is hidden. */
function Notices({
  calendar,
  noCalendars,
  administrator,
}: {
  calendar: ReturnType<typeof useCalendar>
  noCalendars: boolean
  administrator: boolean
}) {
  return (
    <>
      {calendar.isFetching && (
        <span className="calendar__loading" aria-hidden="true" />
      )}
      {calendar.isError && (
        <p role="alert" className="notice notice--danger calendar__notice">
          The calendar could not be read.{' '}
          <button
            type="button"
            className="link-btn"
            onClick={() => void calendar.refetch()}
          >
            Retry
          </button>
        </p>
      )}
      {noCalendars && (
        <div className="notice calendar__notice">
          <NoCalendars administrator={administrator} />
        </div>
      )}
    </>
  )
}

/** The period, the view switch, ‹ Today ›, and the sources pane's toggle. */
function Head({
  view,
  heading,
  announced,
  pane,
  onView,
  onStep,
  onNew,
}: {
  view: View
  heading: [string, string]
  announced: string
  pane: { shown: boolean; toggle: () => void }
  onView: (view: View) => void
  /** -1 and 1 step a period; 0 is today. */
  onStep: (by: -1 | 0 | 1) => void
  onNew: () => void
}) {
  return (
    <header className="page__head calendar__head">
      <button
        type="button"
        className="btn btn--ghost btn--icon btn--sm calendar__toggle"
        aria-label={pane.shown ? 'Hide calendars' : 'Show calendars'}
        title={pane.shown ? 'Hide calendars' : 'Show calendars'}
        aria-expanded={pane.shown}
        aria-controls="cal-sources"
        onClick={pane.toggle}
      >
        <Layers />
      </button>
      <h1>
        {heading[0]} <span className="faint">{heading[1]}</span>
      </h1>
      <p className="sr-only" role="status">
        {announced}
      </p>
      <div className="tabs calendar__views" role="group" aria-label="View">
        {VIEWS.map(([id, label, key]) => (
          <button
            key={id}
            type="button"
            aria-pressed={id === view}
            aria-keyshortcuts={key}
            title={`${label} (${key.toUpperCase()})`}
            onClick={() => onView(id)}
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
          onClick={() => onStep(-1)}
        >
          <ChevronLeft />
        </button>
        <button
          type="button"
          className="btn btn--sm calendar__today"
          aria-keyshortcuts="t"
          title="Today (T)"
          onClick={() => onStep(0)}
        >
          Today
        </button>
        <button
          type="button"
          className="btn btn--ghost btn--icon btn--sm"
          aria-label={`Next ${view}`}
          onClick={() => onStep(1)}
        >
          <ChevronRight />
        </button>
      </div>
      <button
        type="button"
        className="btn btn--ghost btn--icon btn--sm calendar__new"
        aria-label="New event or routine"
        title="New event or routine"
        onClick={onNew}
      >
        <Plus />
      </button>
    </header>
  )
}

function headingOf(
  view: View,
  date: string,
  days: string[],
  phone = false,
): [string, string] {
  // A phone's head shares its line with ‹ Today ›, so the day's name goes short there.
  if (view === 'day')
    return [
      formatDay(date, {
        day: 'numeric',
        month: phone ? 'short' : 'long',
        year: 'numeric',
      }),
      formatDay(date, { weekday: phone ? 'short' : 'long' }),
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

/** The sources pane: remembered with the person's other collapsed panes; a sheet on phones. */
function useSourcesPane(phone: boolean) {
  const preferences = usePreferences()
  const save = useSavePreferences()
  const [sheet, setSheet] = useState(false)
  const collapsed = preferences.data?.collapsed.includes(SOURCES_PANE) ?? false
  const toggle = () => {
    if (phone) return setSheet(!sheet)
    const current = preferences.data
    if (!current) return
    save.mutate({
      ...current,
      collapsed: collapsed
        ? current.collapsed.filter((pane) => pane !== SOURCES_PANE)
        : [...current.collapsed, SOURCES_PANE],
    })
  }
  return { shown: phone ? sheet : !collapsed, toggle }
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

/**
 * A calendar keeps its vendor color; the rest take spare paints, calendars from the front of the
 * list and agents from the back, so an agent's color holds when a calendar is added.
 */
function paintsOf(sources: Source[]) {
  const paints = new Map<string, string>()
  let calendars = 0
  let agents = 0
  for (const source of sources) {
    const vendor =
      source.color && HEX.test(source.color) ? source.color : undefined
    if (source.kind === 'agent')
      paints.set(
        source.id,
        SPARE[SPARE.length - 1 - (agents++ % SPARE.length)] ?? '',
      )
    else
      paints.set(source.id, vendor ?? SPARE[calendars++ % SPARE.length] ?? '')
  }
  return paints
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
