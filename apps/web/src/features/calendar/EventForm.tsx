import { useQuery, useQueryClient } from '@tanstack/react-query'
import { useState } from 'react'
import { api, failure, ok } from '../../api/client'
import type { AgentSummary } from '../../api/types'
import {
  Alert,
  CopyButton,
  Dialog,
  Empty,
  Field,
  Skeleton,
} from '../../components/primitives'
import { RoutineForm } from '../routines/Routines'
import {
  CALENDAR_KEY,
  changeEvent,
  createEvent,
  eventRef,
  useWrite,
} from './api'
import {
  changes,
  cronOf,
  draftOf,
  newDraft,
  timesOf,
  withTimes,
  type Draft,
  type Slot,
  type Times,
} from './model'
import { FULL_DAY, formatDay, type Item, type Source } from './parts'

/** What the editor is open for: something new over a slot, or an item to change. */
export type Editing = { slot: Slot } | { item: Item }

type Context = {
  sources: Source[]
  agents: AgentSummary[]
  zone: string
  onClose: () => void
}

export function Editor({
  editing,
  ...context
}: Context & { editing: Editing }) {
  const title =
    'slot' in editing
      ? 'New'
      : editing.item.kind === 'event'
        ? 'Edit event'
        : 'Edit routine'
  return (
    <Dialog open title={title} onClose={context.onClose}>
      {'slot' in editing ? (
        <NewItem slot={editing.slot} {...context} />
      ) : editing.item.kind === 'event' ? (
        <EventForm item={editing.item} {...context} />
      ) : (
        <RoutineEdit item={editing.item} {...context} />
      )}
    </Dialog>
  )
}

/** Writable calendars, each with the connection and the calendar id the write names. */
function writable(sources: Source[]) {
  return sources.flatMap((source) => {
    const prefix = `calendar:${source.connection ?? ''}:`
    if (source.kind !== 'calendar' || !source.writable || !source.connection)
      return []
    if (!source.id.startsWith(prefix)) return []
    return [
      {
        source,
        connection: source.connection,
        calendar: source.id.slice(prefix.length),
      },
    ]
  })
}

/** Agents the calendar shows routines for: the person's own. */
function ownAgents(sources: Source[], agents: AgentSummary[]) {
  const shown = new Set(sources.map((source) => source.id))
  return agents.filter((agent) => shown.has(`agent:${agent.id}`))
}

function slotWords(slot: Slot) {
  const day = formatDay(slot.day, FULL_DAY)
  if (slot.minute === null) return day
  const hour = String(Math.floor(slot.minute / 60)).padStart(2, '0')
  // A comma, not "at": the day is written in the reader's own language.
  return `${day}, ${hour}:${String(slot.minute % 60).padStart(2, '0')}`
}

/** Event or Routine, like the new-item sheet of iOS Calendar. */
function NewItem({ slot, ...context }: Context & { slot: Slot }) {
  const calendars = writable(context.sources)
  const agents = ownAgents(context.sources, context.agents)
  const [kind, setKind] = useState<'event' | 'routine'>(
    calendars.length || !agents.length ? 'event' : 'routine',
  )
  return (
    <>
      <div className="cal-new__head">
        <div
          className="tabs calendar__views cal-new__kinds"
          role="group"
          aria-label="Kind"
        >
          {(['event', 'routine'] as const).map((each) => (
            <button
              key={each}
              type="button"
              aria-pressed={kind === each}
              onClick={() => setKind(each)}
            >
              {each === 'event' ? 'Event' : 'Routine'}
            </button>
          ))}
        </div>
        <p className="small faint">{slotWords(slot)}</p>
      </div>
      {kind === 'event' ? (
        calendars.length ? (
          <EventForm slot={slot} {...context} />
        ) : (
          <Empty title="No calendar takes new events.">
            Connect a calendar with write access in Platform › Connections, or
            make a routine instead.
          </Empty>
        )
      ) : agents.length ? (
        <RoutineCreate slot={slot} agents={agents} onClose={context.onClose} />
      ) : (
        <Empty title="No agents of yours yet.">
          Create an agent to give it routines.
        </Empty>
      )}
    </>
  )
}

function newEvent(
  chosen: ReturnType<typeof writable>[number] | undefined,
  draft: Draft,
  zone: string,
) {
  return {
    connection: chosen?.connection ?? '',
    calendar: chosen?.calendar ?? '',
    title: draft.title,
    all_day: draft.all_day,
    start: draft.start,
    end: draft.end,
    time_zone: zone,
    location: draft.location.trim() || null,
    description: draft.description.trim() || null,
  }
}

function validate(draft: Draft) {
  if (!draft.title.trim()) return 'Give the event a title.'
  if (
    Number.isNaN(Date.parse(draft.start)) ||
    Number.isNaN(Date.parse(draft.end))
  )
    return 'Pick when the event starts and ends.'
  if (Date.parse(draft.end) <= Date.parse(draft.start))
    return draft.all_day
      ? 'The last day comes before the first.'
      : 'The event ends before it starts.'
  return ''
}

/** A new event over a slot, or an existing one; a repeating one saves for one or all. */
function EventForm({
  item,
  slot,
  sources,
  zone,
  onClose,
}: Context & { item?: Item; slot?: Slot }) {
  const calendars = writable(sources)
  const before = item
    ? draftOf(item)
    : newDraft(slot ?? { day: '', minute: null }, zone)
  const [draft, setDraft] = useState(before)
  const [times, setTimes] = useState<Times>(() => timesOf(before, zone))
  const [calendar, setCalendar] = useState(0)
  const [error, setError] = useState('')
  const write = useWrite()
  const detail = item?.event
  const repeats = Boolean(detail?.series || detail?.occurrence)
  const set = (patch: Partial<Draft>) => setDraft({ ...draft, ...patch })
  const time = (patch: Partial<Times>) => setTimes({ ...times, ...patch })
  // Everything that can throw, the times included, stays inside the try so it shows.
  const save = async (scope: 'one' | 'all') => {
    try {
      const title = draft.title.trim()
      const after = withTimes({ ...draft, title }, times, zone)
      const problem = validate(after)
      setError(problem)
      if (problem) return
      const change = changes(before, after)
      if (detail && !Object.keys(change).length) return onClose()
      await write.mutateAsync(() =>
        detail
          ? changeEvent({ ...eventRef(detail, zone, scope), ...change })
          : createEvent(newEvent(calendars[calendar], after, zone)),
      )
      onClose()
    } catch (error) {
      setError(failure(error))
    }
  }
  return (
    <form
      className="form cal-form"
      onSubmit={(event) => {
        event.preventDefault()
        void save('one')
      }}
    >
      {!item && (
        <Field label="Calendar">
          <select
            value={calendar}
            onChange={(event) => setCalendar(Number(event.target.value))}
          >
            {calendars.map(({ source }, at) => (
              <option key={source.id} value={at}>
                {source.account
                  ? `${source.name} · ${source.account}`
                  : source.name}
              </option>
            ))}
          </select>
        </Field>
      )}
      <Field label="Title">
        <input
          value={draft.title}
          required
          maxLength={1024}
          autoFocus
          onChange={(event) => set({ title: event.target.value })}
        />
      </Field>
      <label className="check">
        <input
          type="checkbox"
          checked={draft.all_day}
          onChange={(event) => set({ all_day: event.target.checked })}
        />
        All day
      </label>
      <TimesFields allDay={draft.all_day} times={times} onChange={time} />
      <Field label="Location">
        <input
          value={draft.location}
          maxLength={1024}
          onChange={(event) => set({ location: event.target.value })}
        />
      </Field>
      <Field label="Notes">
        <textarea
          rows={3}
          maxLength={8192}
          value={draft.description}
          onChange={(event) => set({ description: event.target.value })}
        />
      </Field>
      <Alert>{error}</Alert>
      <div className="actions dialog__actions">
        <button
          type="button"
          className="btn btn--ghost btn--sm"
          onClick={onClose}
        >
          Cancel
        </button>
        {repeats ? (
          <>
            <button
              type="button"
              className="btn btn--sm"
              disabled={write.isPending}
              onClick={() => void save('all')}
            >
              Save all events
            </button>
            <button
              className="btn btn--primary btn--sm"
              disabled={write.isPending}
            >
              Save this event
            </button>
          </>
        ) : (
          <button
            className="btn btn--primary btn--sm"
            disabled={write.isPending}
          >
            {write.isPending ? 'Saving…' : item ? 'Save' : 'Add event'}
          </button>
        )}
      </div>
    </form>
  )
}

function TimesFields({
  allDay,
  times,
  onChange,
}: {
  allDay: boolean
  times: Times
  onChange: (patch: Partial<Times>) => void
}) {
  return (
    <div className="cal-form__times">
      <Field label={allDay ? 'First day' : 'Starts'}>
        <span className="cal-form__when">
          <input
            type="date"
            required
            value={times.startDay}
            onChange={(event) => onChange({ startDay: event.target.value })}
          />
          {!allDay && (
            <input
              type="time"
              required
              aria-label="Start time"
              value={times.startTime}
              onChange={(event) => onChange({ startTime: event.target.value })}
            />
          )}
        </span>
      </Field>
      <Field label={allDay ? 'Last day' : 'Ends'}>
        <span className="cal-form__when">
          <input
            type="date"
            required
            value={times.endDay}
            onChange={(event) => onChange({ endDay: event.target.value })}
          />
          {!allDay && (
            <input
              type="time"
              required
              aria-label="End time"
              value={times.endTime}
              onChange={(event) => onChange({ endTime: event.target.value })}
            />
          )}
        </span>
      </Field>
    </div>
  )
}

/** A webhook secret is shown once, so a routine made with one stays open to copy it. */
function RoutineCreate({
  slot,
  agents,
  onClose,
}: {
  slot: Slot
  agents: AgentSummary[]
  onClose: () => void
}) {
  const client = useQueryClient()
  const [chosen, setChosen] = useState(agents[0]?.id ?? '')
  const [secret, setSecret] = useState('')
  const agent = agents.find((each) => each.id === chosen)
  if (secret)
    return (
      <div className="hook__secret" role="status">
        <p className="small">
          Copy the webhook secret now. It is shown only once and stored only as
          a digest.
        </p>
        <div className="hook__line">
          <code>{secret}</code>
          <CopyButton text={secret} label="Copy webhook secret" />
        </div>
        <div className="actions dialog__actions">
          <button
            type="button"
            className="btn btn--primary btn--sm"
            onClick={onClose}
          >
            Done
          </button>
        </div>
      </div>
    )
  return (
    <>
      {agents.length > 1 && (
        <Field label="Agent">
          <select
            value={chosen}
            onChange={(event) => setChosen(event.target.value)}
          >
            {agents.map((each) => (
              <option key={each.id} value={each.id}>
                {each.name}
              </option>
            ))}
          </select>
        </Field>
      )}
      {agent && (
        <RoutineForm
          key={agent.id}
          agent={agent}
          existing={undefined}
          schedule={cronOf(slot)}
          onDone={(made) => {
            void client.invalidateQueries({ queryKey: CALENDAR_KEY })
            if (made) setSecret(made.secret)
            else onClose()
          }}
        />
      )}
    </>
  )
}

function RoutineEdit({ item, agents, onClose }: Context & { item: Item }) {
  const client = useQueryClient()
  const id = item.routine?.agent ?? ''
  const routines = useQuery({
    queryKey: ['routines', id],
    queryFn: () =>
      ok(
        api.GET('/api/agents/{agent}/routines', {
          params: { path: { agent: id } },
        }),
      ),
    // A routine's revision moves with every save, so an older copy would be refused.
    gcTime: 0,
  })
  const agent = agents.find((each) => each.id === id)
  const routine = routines.data?.find(
    (each) => each.id === item.routine?.routine,
  )
  if (routines.isError) return <Alert>{failure(routines.error)}</Alert>
  if (!routines.data || !agent) return <Skeleton lines={5} />
  if (!routine)
    return (
      <Empty title="This routine is gone.">
        It was deleted since the calendar was read.
      </Empty>
    )
  return (
    <RoutineForm
      agent={agent}
      existing={routine}
      onDone={() => {
        void client.invalidateQueries({ queryKey: CALENDAR_KEY })
        onClose()
      }}
    />
  )
}
