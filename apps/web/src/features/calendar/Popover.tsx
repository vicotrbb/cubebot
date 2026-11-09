import { Link } from '@tanstack/react-router'
import {
  AlignLeft,
  CalendarDays,
  Clock,
  ExternalLink,
  Lock,
  MapPin,
  Play,
  Repeat,
  X,
} from 'lucide-react'
import {
  useEffect,
  useEffectEvent,
  useId,
  useLayoutEffect,
  useRef,
  useState,
} from 'react'
import { api, failure, ok } from '../../api/client'
import { useAgent } from '../../app/queries'
import { Cube } from '../../components/Cube'
import { Alert, confirmAction, Status } from '../../components/primitives'
import { place } from '../agent/AgentHoverCard'
import { removeEvent, useWrite } from './api'
import {
  cadence,
  cronPhrase,
  digestDay,
  eventRef,
  formatRange,
  formatTime,
  localDay,
  placeOf,
  type Merged,
} from './model'
import {
  FULL_DAY,
  formatDay,
  Glyph,
  titleOf,
  type Item,
  type Look,
  type Point,
} from './parts'

/** The item a click chose, where, and on which of its days. */
export type Selection = {
  item: Merged
  at: Point
  day: string
  opener: HTMLElement | null
}

type Actions = {
  look: Look
  /** Closes the card; `back` returns focus to the item it opened from. */
  close: (back: boolean) => void
  onEdit: (item: Item) => void
}

/**
 * An item's details beside the point that chose it, like a macOS Calendar popover; on a phone a
 * bottom sheet, which is where iOS puts a card the thumb has to reach.
 */
export function Popover({
  selection,
  sheet,
  look,
  onClose,
  onEdit,
}: {
  selection: Selection
  sheet: boolean
  look: Look
  onClose: () => void
  onEdit: (item: Item) => void
}) {
  const ref = useRef<HTMLDialogElement>(null)
  const title = useId()
  const { item, at, opener } = selection
  // The item may have been read again since, so the selected one stands in for the opener.
  const focusItem = () =>
    (opener?.isConnected
      ? opener
      : document.querySelector<HTMLElement>('.calendar [data-selected]')
    )?.focus()
  const close = (back: boolean) => {
    if (back) focusItem()
    onClose()
  }
  // The editor returns focus to whatever had it when it opened, so that is the item; the sheet
  // is modal and keeps the item inert until it closes.
  const edit = (item: Item) => {
    ref.current?.close()
    focusItem()
    onEdit(item)
  }
  useLayoutEffect(() => {
    const dialog = ref.current
    if (!dialog) return
    if (sheet) dialog.showModal()
    else {
      dialog.show()
      place(dialog, at.x, at.y)
    }
    dialog.focus()
  }, [sheet, at])
  useDismiss(ref, sheet, close)
  const day = item.kind === 'digest' ? digestDay(item, selection.day) : item
  return (
    <dialog
      ref={ref}
      className="cal-pop"
      data-sheet={sheet || undefined}
      aria-labelledby={title}
      tabIndex={-1}
      style={{ '--c': look.paint(item) } as React.CSSProperties}
      onCancel={(event) => {
        event.preventDefault()
        close(true)
      }}
      onClick={(event) => {
        // The sheet's backdrop is the dialog itself.
        if (event.target === event.currentTarget && sheet) close(true)
      }}
    >
      <header className="cal-pop__head">
        <span className="cal-pop__swatch" aria-hidden="true" />
        <h2 id={title}>{titleOf(item)}</h2>
        <button
          type="button"
          className="btn btn--ghost btn--icon btn--sm"
          aria-label="Close"
          onClick={() => close(true)}
        >
          <X />
        </button>
      </header>
      {item.kind === 'event' && (
        <EventBody item={item} look={look} close={close} onEdit={edit} />
      )}
      {item.kind === 'routine' && (
        <RoutineBody item={item} look={look} close={close} onEdit={edit} />
      )}
      {item.kind === 'run' && <RunBody item={item} look={look} />}
      {item.kind === 'digest' && <DigestBody item={day} look={look} />}
    </dialog>
  )
}

/** Escape and a press outside close the floating card; the sheet is modal and has its own. */
function useDismiss(
  ref: React.RefObject<HTMLDialogElement | null>,
  sheet: boolean,
  close: (back: boolean) => void,
) {
  const dismiss = useEffectEvent(close)
  useEffect(() => {
    if (sheet) return
    // A confirmation asked from the card is another dialog; it keeps the card open.
    const inOther = (target: EventTarget | null) =>
      target instanceof Element &&
      !ref.current?.contains(target) &&
      target.closest('dialog') !== null
    const press = (event: PointerEvent) => {
      const target = event.target as Node
      if (!ref.current?.contains(target) && !inOther(target)) dismiss(false)
    }
    const key = (event: KeyboardEvent) => {
      if (event.key === 'Escape' && !inOther(event.target)) dismiss(true)
    }
    document.addEventListener('pointerdown', press)
    document.addEventListener('keydown', key)
    return () => {
      document.removeEventListener('pointerdown', press)
      document.removeEventListener('keydown', key)
    }
  }, [ref, sheet])
}

function Row({
  icon: Icon,
  label,
  children,
}: {
  icon: typeof Clock
  label: string
  children: React.ReactNode
}) {
  return (
    <div className="cal-pop__row">
      <Icon aria-label={label} role="img" />
      <div>{children}</div>
    </div>
  )
}

/** "Tuesday, November 10" over "10:00 – 11:00", or the days an item spans. */
function When({ item, zone }: { item: Item; zone: string }) {
  const days = localDay(item, zone)
  const first = days[0] ?? ''
  const last = days.at(-1) ?? first
  const short = { weekday: 'short', day: 'numeric', month: 'short' } as const
  if (first === last)
    return (
      <Row icon={Clock} label="When">
        <span>{formatDay(first, FULL_DAY)}</span>
        <span className="cal-pop__faint tabular">
          {formatRange(item, zone)}
        </span>
      </Row>
    )
  const at = (instant: string | null | undefined) =>
    item.all_day || !instant ? '' : `, ${formatTime(instant, zone)}`
  return (
    <Row icon={Clock} label="When">
      <span className="tabular">
        {formatDay(first, short)}
        {at(item.start)} – {formatDay(last, short)}
        {at(item.end)}
      </span>
      {item.all_day && <span className="cal-pop__faint">All day</span>}
    </Row>
  )
}

const WEB = /^https?:\/\//i

function EventBody(props: Actions & { item: Item }) {
  return (
    <>
      <EventDetails item={props.item} look={props.look} />
      <EventActions {...props} />
    </>
  )
}

const repeating = (item: Item) =>
  Boolean(item.event?.series || item.event?.occurrence)

function EventDetails({ item, look }: { item: Item; look: Look }) {
  const detail = item.event
  return (
    <div className="cal-pop__body">
      <When item={item} zone={look.zone} />
      {repeating(item) && (
        <Row icon={Repeat} label="Repeats">
          <span>Repeating event</span>
        </Row>
      )}
      {detail?.location && (
        <Row icon={MapPin} label="Location">
          <span className="cal-pop__wrap">{detail.location}</span>
        </Row>
      )}
      <Row icon={CalendarDays} label="Calendar">
        <span>{look.sourceName(item)}</span>
        {!item.editable && (
          <span className="cal-pop__faint cal-pop__lock">
            <Lock aria-hidden="true" /> Read-only here
          </span>
        )}
      </Row>
      {detail?.description && (
        <Row icon={AlignLeft} label="Notes">
          <p className="cal-pop__notes">{detail.description}</p>
        </Row>
      )}
    </div>
  )
}

/** Open, Edit and Delete; deleting a repeating event asks for this one or all, as Apple Calendar does. */
function EventActions({ item, look, close, onEdit }: Actions & { item: Item }) {
  const write = useWrite()
  const [asking, setAsking] = useState(false)
  const [error, setError] = useState('')
  const detail = item.event
  const link = detail?.link && WEB.test(detail.link) ? detail.link : null
  const remove = async (scope: 'one' | 'all') => {
    if (!detail) return
    setError('')
    try {
      await write.mutateAsync(() =>
        removeEvent(eventRef(detail, look.zone, scope)),
      )
      close(false)
    } catch (error) {
      setAsking(false)
      setError(failure(error))
    }
  }
  const ask = async () => {
    if (repeating(item)) return setAsking(true)
    const yes = await confirmAction({
      title: `Delete ${item.title}?`,
      body: `It is removed from ${look.sourceName(item)}.`,
      action: 'Delete',
    })
    if (yes) await remove('one')
  }
  if (asking)
    return (
      <div
        className="cal-pop__ask"
        role="group"
        aria-label="Delete a repeating event"
      >
        <p>This event repeats. Delete only this one, or every one?</p>
        <div className="cal-pop__actions">
          <button
            type="button"
            className="btn btn--ghost btn--sm"
            onClick={() => setAsking(false)}
          >
            Cancel
          </button>
          <button
            type="button"
            className="btn btn--danger btn--sm"
            disabled={write.isPending}
            onClick={() => void remove('all')}
          >
            All events
          </button>
          <button
            type="button"
            className="btn btn--danger btn--sm"
            disabled={write.isPending}
            autoFocus
            onClick={() => void remove('one')}
          >
            This event
          </button>
        </div>
      </div>
    )
  if (!link && !(item.editable && detail)) return null
  return (
    <>
      <Alert>{error}</Alert>
      <div className="cal-pop__actions">
        {link && (
          <a
            className="btn btn--ghost btn--sm cal-pop__lead"
            href={link}
            target="_blank"
            rel="noopener noreferrer"
          >
            <ExternalLink aria-hidden="true" /> Open original
          </a>
        )}
        {item.editable && detail && (
          <>
            <button
              type="button"
              className="btn btn--ghost btn--sm cal-pop__delete"
              disabled={write.isPending}
              onClick={() => void ask()}
            >
              {write.isPending ? 'Deleting…' : 'Delete'}
            </button>
            <button
              type="button"
              className="btn btn--sm"
              onClick={() => onEdit(item)}
            >
              Edit
            </button>
          </>
        )}
      </div>
    </>
  )
}

function zoneNote(zone: string, viewer: string) {
  return zone === viewer ? null : `${placeOf(zone)} time`
}

function RoutineBody({ item, look, close, onEdit }: Actions & { item: Item }) {
  const write = useWrite()
  const [error, setError] = useState('')
  const [notice, setNotice] = useState('')
  const routine = item.routine
  if (!routine) return null
  const path = {
    params: { path: { agent: routine.agent, routine: routine.routine } },
  }
  const act = async (action: () => Promise<unknown>) => {
    setError('')
    setNotice('')
    try {
      await write.mutateAsync(action)
    } catch (error) {
      setError(failure(error))
      throw error
    }
  }
  const run = () =>
    act(async () => {
      const ran = await ok(
        api.POST('/api/agents/{agent}/routines/{routine}/runs', {
          ...path,
          body: { request_id: crypto.randomUUID() },
        }),
      )
      setNotice(
        ran.turn
          ? `${item.title} is queued in the conversation.`
          : `A run of ${item.title} is already waiting; nothing new was queued.`,
      )
    }).catch(() => undefined)
  const remove = async () => {
    const yes = await confirmAction({
      title: `Delete ${item.title}?`,
      body: `${look.sourceName(item)} stops running it. Past runs stay in the conversation.`,
      action: 'Delete',
    })
    if (!yes) return
    await act(() =>
      ok(api.DELETE('/api/agents/{agent}/routines/{routine}', path)),
    )
      .then(() => close(false))
      .catch(() => undefined)
  }
  return (
    <>
      <div className="cal-pop__body">
        <Row icon={Clock} label="Next run">
          <span>{formatDay(selectedDay(item, look.zone), FULL_DAY)}</span>
          <span className="cal-pop__faint tabular">
            {formatTime(item.start, look.zone)}
          </span>
        </Row>
        <Row icon={Repeat} label="Schedule">
          <span>
            {cronPhrase(routine.schedule) ?? <code>{routine.schedule}</code>}
          </span>
          {zoneNote(routine.time_zone, look.zone) && (
            <span className="cal-pop__faint">
              {zoneNote(routine.time_zone, look.zone)}
            </span>
          )}
        </Row>
        <AgentRow item={item} look={look} />
      </div>
      <Alert>{error}</Alert>
      <Status>{notice}</Status>
      <div className="cal-pop__actions">
        <button
          type="button"
          className="btn btn--ghost btn--sm cal-pop__delete cal-pop__lead"
          disabled={write.isPending}
          onClick={() => void remove()}
        >
          Delete
        </button>
        <button
          type="button"
          className="btn btn--sm"
          onClick={() => onEdit(item)}
        >
          Edit
        </button>
        <button
          type="button"
          className="btn btn--primary btn--sm"
          disabled={write.isPending}
          onClick={() => void run()}
        >
          <Play aria-hidden="true" /> Run now
        </button>
      </div>
    </>
  )
}

const selectedDay = (item: Item, zone: string) => localDay(item, zone)[0] ?? ''

/** The agent's cube and a way to its routines. */
function AgentRow({ item, look }: { item: Item; look: Look }) {
  const id = item.routine?.agent ?? ''
  const { agent } = useAgent(id)
  return (
    <div className="cal-pop__row">
      <Cube avatar={agent?.avatar ?? null} size={16} />
      <div>
        <span>{look.sourceName(item)}</span>
        <Link
          to="/agents/$agentId/routines"
          params={{ agentId: id }}
          className="cal-pop__link"
        >
          All routines
        </Link>
      </div>
    </div>
  )
}

const OUTCOMES: Record<string, string> = {
  complete: 'Done',
  failed: 'Failed',
  cancelled: 'Cancelled',
}

function duration(item: Item) {
  if (!item.end) return null
  const seconds = Math.round(
    (Date.parse(item.end) - Date.parse(item.start)) / 1000,
  )
  if (seconds < 60) return `${seconds} s`
  const minutes = Math.round(seconds / 60)
  return minutes < 60
    ? `${minutes} min`
    : `${Math.floor(minutes / 60)} h ${minutes % 60} min`
}

function RunBody({ item, look }: { item: Item; look: Look }) {
  const routine = item.routine
  const took = duration(item)
  return (
    <div className="cal-pop__body">
      <div className="cal-pop__row cal-pop__outcome">
        <Glyph item={item} />
        <div>
          <span>{OUTCOMES[routine?.outcome ?? ''] ?? 'Not finished'}</span>
          {took && <span className="cal-pop__faint">Took {took}</span>}
        </div>
      </div>
      <Row icon={Clock} label="Started">
        <span>{formatDay(selectedDay(item, look.zone), FULL_DAY)}</span>
        <span className="cal-pop__faint tabular">
          {formatTime(item.start, look.zone)}
        </span>
      </Row>
      <AgentRow item={item} look={look} />
      {routine?.turn && (
        <div className="cal-pop__actions">
          <Link
            to="/agents/$agentId/thread/$turnId"
            params={{ agentId: routine.agent, turnId: routine.turn }}
            className="btn btn--sm"
          >
            Open the run
          </Link>
        </div>
      )}
    </div>
  )
}

function DigestBody({ item, look }: { item: Item; look: Look }) {
  const routine = item.routine
  const runs = routine?.runs ?? 0
  return (
    <div className="cal-pop__body">
      <Row icon={Repeat} label="Cadence">
        <span>Runs {cadence(routine?.schedule ?? '')}</span>
        <span className="cal-pop__faint">
          {runs === 1 ? '1 run' : `${runs} runs`} on{' '}
          {formatDay(item.start, FULL_DAY)}
        </span>
      </Row>
      <AgentRow item={item} look={look} />
    </div>
  )
}
