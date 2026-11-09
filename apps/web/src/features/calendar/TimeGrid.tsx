import { useLayoutEffect, useMemo, useRef, type MouseEvent } from 'react'
import {
  formatTime,
  layoutDay,
  weekLanes,
  type Bar,
  type Placed,
} from './model'
import {
  FULL_DAY,
  formatDay,
  Glyph,
  itemProps,
  titleOf,
  type Item,
  type Look,
  type Slot,
} from './parts'

const DAY = 1440
// Below these many minutes an item has room for one line, or for its title and time only.
const ONE_LINE = 50
const TWO_LINES = 80
// How far a stacked item steps right of the one under it.
const INDENT = 10
const HOURS = Array.from({ length: 23 }, (_, at) => at + 1)

/** Week and Day: an all-day band over the hours, items placed by wall-clock minute. */
export function TimeGrid({
  days,
  items,
  look,
  today,
  date,
  now,
  onOpen,
  onCreate,
}: {
  days: string[]
  items: Item[]
  look: Look
  today: string
  date: string
  now: number
  onOpen: (day: string) => void
  onCreate?: ((slot: Slot) => void) | undefined
}) {
  const allDay = useMemo(
    () =>
      weekLanes(
        items.filter((item) => item.all_day),
        days,
        look.zone,
        Infinity,
      ).bars,
    [items, days, look.zone],
  )
  const lanes = Math.max(1, ...allDay.map((bar) => bar.lane + 1))
  const showsToday = days.includes(today)
  const scroller = useInitialScroll(showsToday ? now : 8 * 60)
  return (
    <div
      className="timegrid"
      ref={scroller}
      style={
        {
          '--days': days.length,
          '--lanes': lanes,
        } as React.CSSProperties
      }
      data-days={days.length}
    >
      <div className="timegrid__corner" aria-hidden="true" />
      {days.map((day) => (
        <button
          key={day}
          type="button"
          className="timegrid__day"
          data-today={day === today || undefined}
          data-chosen={(day === date && days.length > 1) || undefined}
          aria-label={`${formatDay(day, FULL_DAY)}${day === today ? ', today' : ''}, open the day`}
          disabled={days.length === 1}
          onClick={() => onOpen(day)}
        >
          <span className="timegrid__weekday">
            {formatDay(day, { weekday: 'short' })}
          </span>
          <span className="timegrid__date">{Number(day.slice(8))}</span>
        </button>
      ))}
      <div className="timegrid__allday-label">all-day</div>
      <div className="timegrid__allday">
        {allDay.map((bar) => (
          <AllDay key={bar.item.id} bar={bar} look={look} />
        ))}
      </div>
      <div className="timegrid__hours" aria-hidden="true">
        {HOURS.filter(
          // The current time's label stands in for an hour it would cover.
          (hour) => !showsToday || Math.abs(hour * 60 - now) > 20,
        ).map((hour) => (
          <span key={hour} style={{ top: `${(hour / 24) * 100}%` }}>
            {String(hour).padStart(2, '0')}:00
          </span>
        ))}
        {showsToday && (
          <span className="timegrid__now-label" style={{ top: at(now) }}>
            {clock(now)}
          </span>
        )}
      </div>
      {days.map((day) => (
        <Column
          key={day}
          day={day}
          items={items}
          look={look}
          now={showsToday ? now : null}
          today={day === today}
          onCreate={onCreate}
        />
      ))}
    </div>
  )
}

const at = (minute: number) => `${(minute / DAY) * 100}%`
const clock = (minute: number) =>
  `${String(Math.floor(minute / 60)).padStart(2, '0')}:${String(Math.floor(minute % 60)).padStart(2, '0')}`

/** Opens on the current time, or the working morning, once per view. */
function useInitialScroll(minute: number) {
  const scroller = useRef<HTMLDivElement>(null)
  const first = useRef(minute)
  useLayoutEffect(() => {
    const grid = scroller.current
    const column = grid?.querySelector<HTMLElement>('.timegrid__col')
    if (!grid || !column) return
    // The sticky head covers the grid's top, so scrolling by a minute's offset shows that minute under it.
    grid.scrollTop =
      (Math.max(0, first.current - 90) / DAY) * column.offsetHeight
  }, [])
  return scroller
}

function Column({
  day,
  items,
  look,
  now,
  today,
  onCreate,
}: {
  day: string
  items: Item[]
  look: Look
  now: number | null
  today: boolean
  onCreate?: ((slot: Slot) => void) | undefined
}) {
  const placed = useMemo(
    () => layoutDay(items, day, look.zone),
    [items, day, look.zone],
  )
  const create = (event: MouseEvent<HTMLDivElement>) => {
    if (!onCreate || event.target !== event.currentTarget) return
    const box = event.currentTarget.getBoundingClientRect()
    const minute = ((event.clientY - box.top) / box.height) * DAY
    onCreate({ day, minute: Math.floor(minute / 30) * 30 })
  }
  return (
    <div
      role="group"
      aria-label={formatDay(day, FULL_DAY)}
      className="timegrid__col"
      data-weekend={
        [0, 6].includes(new Date(`${day}T12:00:00Z`).getUTCDay()) || undefined
      }
      data-creates={onCreate ? true : undefined}
      onClick={create}
    >
      {placed.map((place) => (
        <GridItem key={place.item.id} place={place} look={look} />
      ))}
      {now !== null && (
        <span
          className="timegrid__now"
          data-faint={!today || undefined}
          style={{ top: at(now) }}
          aria-hidden="true"
        />
      )}
    </div>
  )
}

function AllDay({ bar, look }: { bar: Bar; look: Look }) {
  const props = itemProps(bar.item, look)
  return (
    <button
      {...props}
      className="cal-chip"
      style={{
        ...props.style,
        gridColumn: `${bar.first + 1} / ${bar.last + 2}`,
        gridRow: bar.lane + 1,
      }}
    >
      <Glyph item={bar.item} />
      <span className="cal-chip__title">{titleOf(bar.item)}</span>
    </button>
  )
}

function GridItem({ place, look }: { place: Placed; look: Look }) {
  const { item, top, height, column, columns, indent } = place
  const props = itemProps(item, look)
  const short = height < ONE_LINE
  const roomy = height >= TWO_LINES
  return (
    <button
      {...props}
      className="cal-block"
      data-short={short || undefined}
      data-roomy={roomy || undefined}
      style={{
        ...props.style,
        top: at(top),
        height: `calc(${at(height)} - 1px)`,
        left: `calc(${(column / columns) * 100}% + ${indent * INDENT + 1}px)`,
        width: `calc(${100 / columns}% - ${indent * INDENT + 3}px)`,
      }}
    >
      <span className="cal-block__title">
        <Glyph item={item} />
        {titleOf(item)}
      </span>
      <time className="cal-block__time" dateTime={item.start}>
        {formatTime(item.start, look.zone)}
      </time>
      {roomy && item.event?.location && (
        <span className="cal-block__where">{item.event.location}</span>
      )}
    </button>
  )
}
