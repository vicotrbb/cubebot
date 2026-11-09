import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type Ref,
} from 'react'
import {
  busyOn,
  formatTime,
  localDay,
  weekLanes,
  type Bar,
  type Merged,
  type Slot,
} from './model'
import {
  FULL_DAY,
  formatDay,
  Glyph,
  itemProps,
  titleOf,
  type Item,
  type Look,
} from './parts'

// Must match calendar.css: the day number's row with the cell's foot, and one lane with its gap.
const HEAD = 30
const LANE = 20

type Days = {
  today: string
  date: string
  onPick: (day: string) => void
  onOpen: (day: string) => void
  onCreate?: ((slot: Slot) => void) | undefined
}

/** The month as whole weeks; on a phone each day shows dots and the chosen day is listed below. */
export function MonthView({
  days,
  month,
  items,
  look,
  compact,
  ...pick
}: Days & {
  days: string[]
  month: string
  items: Item[]
  look: Look
  compact: boolean
}) {
  const weeks = useMemo(
    () =>
      Array.from({ length: days.length / 7 }, (_, at) =>
        days.slice(at * 7, at * 7 + 7),
      ),
    [days],
  )
  const [lanes, rowRef] = useLanes()
  useFocusFollows(pick.date)
  return (
    <div
      className="month"
      role="grid"
      aria-label={formatDay(`${month}-01`, { month: 'long', year: 'numeric' })}
      data-compact={compact || undefined}
    >
      <div className="month__weekdays" role="row">
        {weeks[0]?.map((day) => (
          <div role="columnheader" className="month__weekday" key={day}>
            <abbr title={formatDay(day, { weekday: 'long' })}>
              {formatDay(day, { weekday: compact ? 'narrow' : 'short' })}
            </abbr>
          </div>
        ))}
      </div>
      {weeks.map((week, at) => (
        <Week
          key={week[0]}
          week={week}
          month={month}
          items={items}
          look={look}
          lanes={compact ? Infinity : lanes}
          compact={compact}
          rowRef={at === 0 ? rowRef : undefined}
          {...pick}
        />
      ))}
    </div>
  )
}

/** When the keyboard moves the chosen day, focus moves with it. */
function useFocusFollows(date: string) {
  const shown = useRef(date)
  useEffect(() => {
    if (shown.current === date) return
    shown.current = date
    const active = document.activeElement
    // A focused day that scrolled out of the month leaves focus on the body.
    const inGrid =
      active === document.body || active?.classList.contains('month__day')
    if (inGrid)
      document
        .querySelector<HTMLElement>(`.month__day[data-day="${date}"]`)
        ?.focus()
  }, [date])
}

/** How many lanes a week row holds at its current height; the first row changes with the month. */
function useLanes() {
  const [lanes, setLanes] = useState(3)
  const rowRef = useCallback((row: HTMLDivElement | null) => {
    if (!row) return
    const measure = () => {
      if (row.isConnected)
        setLanes(Math.max(1, Math.floor((row.clientHeight - HEAD) / LANE)))
    }
    // Measured as the row attaches, so the first paint already has the right lanes.
    measure()
    const observer = new ResizeObserver(measure)
    observer.observe(row)
    return () => observer.disconnect()
  }, [])
  return [lanes, rowRef] as const
}

function Week({
  week,
  month,
  items,
  look,
  lanes,
  compact,
  rowRef,
  ...days
}: Days & {
  week: string[]
  month: string
  items: Item[]
  look: Look
  lanes: number
  compact: boolean
  rowRef: Ref<HTMLDivElement> | undefined
}) {
  const { bars, more } = useMemo(
    () => weekLanes(items, week, look.zone, lanes),
    [items, week, look.zone, lanes],
  )
  return (
    <div className="month__week" role="row" ref={rowRef}>
      {week.map((day, column) => (
        <Day
          key={day}
          day={day}
          outside={!day.startsWith(month)}
          compact={compact}
          {...days}
        >
          {compact ? (
            <Dots bars={bars} column={column} look={look} />
          ) : (
            <Lanes
              bars={bars}
              column={column}
              week={week}
              more={more[column] ?? 0}
              look={look}
              onMore={() => days.onOpen(day)}
            />
          )}
        </Day>
      ))}
    </div>
  )
}

function Day({
  day,
  outside,
  today,
  date,
  onPick,
  onOpen,
  onCreate,
  compact,
  children,
}: Days & {
  day: string
  outside: boolean
  compact: boolean
  children: React.ReactNode
}) {
  const weekday = new Date(`${day}T12:00:00Z`).getUTCDay()
  const number =
    day.endsWith('-01') && !compact
      ? formatDay(day, { day: 'numeric', month: 'short' })
      : String(Number(day.slice(8)))
  return (
    <div
      role="gridcell"
      className="month__day"
      data-day={day}
      data-outside={outside || undefined}
      data-today={day === today || undefined}
      data-weekend={weekday === 0 || weekday === 6 || undefined}
      aria-selected={day === date}
      tabIndex={day === date ? 0 : -1}
      onClick={() => onPick(day)}
      onDoubleClick={onCreate && (() => onCreate({ day, minute: null }))}
      onKeyDown={(event) => {
        if (event.key === 'Enter' && event.target === event.currentTarget)
          onOpen(day)
      }}
    >
      <button
        type="button"
        className="month__num"
        tabIndex={-1}
        aria-label={`${formatDay(day, FULL_DAY)}${day === today ? ', today' : ''}, open the day`}
        onClick={(event) => {
          event.stopPropagation()
          onOpen(day)
        }}
      >
        {number}
      </button>
      {children}
    </div>
  )
}

function Lanes({
  bars,
  column,
  week,
  more,
  look,
  onMore,
}: {
  bars: Bar[]
  column: number
  week: string[]
  more: number
  look: Look
  onMore: () => void
}) {
  const here = bars.filter((bar) => bar.first <= column && column <= bar.last)
  const depth = Math.max(-1, ...here.map((bar) => bar.lane)) + 1
  return (
    <div className="month__lanes">
      {Array.from({ length: depth }, (_, lane) => {
        const bar = here.find((each) => each.lane === lane)
        // A bar is drawn once, from its first day here, over the days it spans.
        if (bar?.first !== column)
          return <span key={lane} className="month__gap" aria-hidden="true" />
        return <MonthItem key={lane} bar={bar} week={week} look={look} />
      })}
      {more > 0 && (
        <button
          type="button"
          className="month__more"
          onClick={(event) => {
            event.stopPropagation()
            onMore()
          }}
        >
          {more} more
        </button>
      )}
    </div>
  )
}

function MonthItem({
  bar,
  week,
  look,
}: {
  bar: Bar
  week: string[]
  look: Look
}) {
  const { item, first, last } = bar
  const shape = item.all_day || last > first ? 'bar' : 'line'
  const props = itemProps(item, look)
  const covered = shape === 'bar' ? localDay(item, look.zone) : []
  return (
    <button
      {...props}
      className={`month__item month__item--${shape}`}
      data-before={(covered[0] ?? '') < (week[0] ?? '') || undefined}
      data-after={(covered.at(-1) ?? '') > (week[6] ?? '') || undefined}
      style={
        { ...props.style, '--span': last - first + 1 } as React.CSSProperties
      }
    >
      {shape === 'line' && item.kind === 'event' && (
        <span className="month__dot" aria-hidden="true" />
      )}
      <Glyph item={item} />
      <span className="month__title">{titleOf(item)}</span>
      {shape === 'line' && (
        <time className="month__time" dateTime={item.start}>
          {formatTime(item.start, look.zone)}
        </time>
      )}
    </button>
  )
}

function Dots({
  bars,
  column,
  look,
}: {
  bars: Bar[]
  column: number
  look: Look
}) {
  const here = busyOn(bars, column)
  const paints = [...new Set(here.map((bar) => look.paint(bar.item)))].slice(
    0,
    3,
  )
  if (!here.length) return null
  return (
    <span className="month__dots">
      {paints.map((paint) => (
        <span key={paint} style={{ '--c': paint } as React.CSSProperties} />
      ))}
      <span className="sr-only">
        {here.length === 1 ? '1 item' : `${here.length} items`}
      </span>
    </span>
  )
}

/** The chosen day's items as a list, under the phone's month. */
export function Agenda({
  day,
  items,
  look,
}: {
  day: string
  items: Item[]
  look: Look
}) {
  const here = items.filter((item) => localDay(item, look.zone).includes(day))
  return (
    <section className="agenda" aria-labelledby="agenda-head">
      <h2 id="agenda-head">{formatDay(day, FULL_DAY)}</h2>
      {here.length === 0 ? (
        <p className="agenda__none">Nothing scheduled.</p>
      ) : (
        <ul>
          {here.map((item) => (
            <li key={item.id}>
              <AgendaRow item={item} day={day} look={look} />
            </li>
          ))}
        </ul>
      )}
    </section>
  )
}

function AgendaRow({
  item,
  day,
  look,
}: {
  item: Merged
  day: string
  look: Look
}) {
  const timed = !item.all_day
  const runs = item.days?.find((each) => each.start === day)?.routine?.runs
  return (
    <button {...itemProps(item, look)} className="agenda__item">
      <span className="agenda__time tabular">
        {timed ? formatTime(item.start, look.zone) : 'All day'}
        {timed && item.end && <span>{formatTime(item.end, look.zone)}</span>}
      </span>
      <span className="agenda__bar" aria-hidden="true" />
      <span className="agenda__text">
        <span className="agenda__title">
          <Glyph item={item} />
          {titleOf(item)}
        </span>
        <span className="agenda__source">
          {[
            look.sourceName(item),
            item.event?.location,
            item.kind === 'digest' && `${runs ?? 0} runs`,
          ]
            .filter(Boolean)
            .join(' · ')}
        </span>
      </span>
    </button>
  )
}
