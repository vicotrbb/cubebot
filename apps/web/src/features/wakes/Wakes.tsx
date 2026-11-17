import { useQuery, useQueryClient } from '@tanstack/react-query'
import { AlarmClock, Repeat } from 'lucide-react'
import { useState } from 'react'
import { api, failure, ok } from '../../api/client'
import type { components } from '../../api/schema'
import type { AgentSummary } from '../../api/types'
import {
  Alert,
  confirmAction,
  Empty,
  Field,
  Skeleton,
} from '../../components/primitives'
import { scheduleWords } from '../calendar/model'
import { SCHEDULES, useViewerZone, ZoneField } from '../routines/Routines'
import '../platform/platform.css'
import './wakes.css'

type Wake = components['schemas']['Wake']
type Mode = 'once' | 'repeats'

export function Wakes({
  agent,
  editable,
}: {
  agent: AgentSummary
  editable: boolean
}) {
  const client = useQueryClient()
  const key = ['wakes', agent.id]
  const wakes = useQuery({
    queryKey: key,
    queryFn: () =>
      ok(
        api.GET('/api/agents/{agent}/wakes', {
          params: { path: { agent: agent.id } },
        }),
      ),
    refetchInterval: 30_000,
  })
  const [editing, setEditing] = useState<Wake | 'new' | null>(null)
  const [error, setError] = useState('')
  const refresh = () => client.invalidateQueries({ queryKey: key })
  async function cancel(wake: Wake) {
    const yes = await confirmAction({
      title: 'Cancel this wake?',
      body: `${agent.name} will not be woken for it. Messages it already sent stay in the chat.`,
      action: 'Cancel wake',
    })
    if (!yes) return
    setError('')
    try {
      await ok(
        api.DELETE('/api/agents/{agent}/wakes/{wake}', {
          params: { path: { agent: agent.id, wake: wake.id } },
        }),
      )
      await refresh()
    } catch (caught) {
      setError(failure(caught))
    }
  }
  const list = wakes.data ?? []
  return (
    <>
      <section className="panel__section">
        <h2>Wakes</h2>
        <p className="panel__lead">
          A wake is a note {agent.name} leaves itself. At the time set, its
          instructions arrive as a message in this chat and it picks the work
          back up. Unlike routines, wakes stay off the calendar.
        </p>
        {editable && !editing && (
          <div className="actions">
            <button
              type="button"
              className="btn btn--primary btn--sm"
              onClick={() => setEditing('new')}
            >
              New wake
            </button>
          </div>
        )}
        {editing && (
          <WakeForm
            agent={agent}
            existing={editing === 'new' ? undefined : editing}
            onDone={() => {
              setEditing(null)
              void refresh()
            }}
          />
        )}
        <Alert>{error}</Alert>
      </section>
      <section className="panel__section">
        {wakes.isPending && <Skeleton lines={3} />}
        {wakes.isError && <Alert>{failure(wakes.error)}</Alert>}
        {wakes.isSuccess && list.length === 0 && (
          <Empty title="No wakes set.">
            When {agent.name} has to wait for a deploy, a reply or a date, it
            sets a wake and comes back on time. You can set one for it too.
          </Empty>
        )}
        <ul className="wakes">
          {list.map((wake) => (
            <WakeRow
              key={wake.id}
              agent={agent}
              wake={wake}
              editable={editable && editing === null}
              onEdit={() => setEditing(wake)}
              onCancel={() => void cancel(wake)}
            />
          ))}
        </ul>
      </section>
    </>
  )
}

function WakeRow({
  agent,
  wake,
  editable,
  onEdit,
  onCancel,
}: {
  agent: AgentSummary
  wake: Wake
  editable: boolean
  onEdit: () => void
  onCancel: () => void
}) {
  const viewer = useViewerZone()
  const Icon = wake.schedule ? Repeat : AlarmClock
  return (
    <li className="wake">
      <span className="wake__icon" aria-hidden="true">
        <Icon />
      </span>
      <div className="wake__body">
        <p className="wake__when">
          {wake.schedule
            ? scheduleWords(wake.schedule, wake.time_zone, null, viewer)
            : moment(wake.next_at, viewer)}
          <span className="wake__until">{until(wake.next_at)}</span>
        </p>
        <p className="wake__text">{wake.instructions}</p>
        <div className="wake__foot">
          <span className="faint small">
            {wake.set_by === 'agent'
              ? `Set by ${agent.name}`
              : 'Set by a person'}
            {wake.schedule && ` · next ${moment(wake.next_at, viewer)}`}
          </span>
          {editable && (
            <span className="wake__actions">
              <button type="button" className="link-btn small" onClick={onEdit}>
                Edit
              </button>
              <button
                type="button"
                className="link-btn small"
                onClick={onCancel}
              >
                Cancel wake
              </button>
            </span>
          )}
        </div>
      </div>
    </li>
  )
}

function WakeForm({
  agent,
  existing,
  onDone,
}: {
  agent: AgentSummary
  existing: Wake | undefined
  onDone: () => void
}) {
  const [mode, setMode] = useState<Mode>(
    existing?.schedule ? 'repeats' : 'once',
  )
  const [at, setAt] = useState(() =>
    existing && !existing.schedule
      ? local(new Date(existing.next_at))
      : local(soon()),
  )
  const [cron, setCron] = useState(existing?.schedule ?? SCHEDULES[0][1])
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  async function save(form: HTMLFormElement) {
    const data = new FormData(form)
    const instructions = String(data.get('instructions') ?? '').trim()
    if (mode === 'once' && new Date(at).getTime() <= Date.now())
      return setError('Pick a time in the future.')
    const wake = {
      instructions,
      at: mode === 'once' ? new Date(at).toISOString() : null,
      schedule: mode === 'repeats' ? cron.trim() : null,
      time_zone: mode === 'repeats' ? String(data.get('time_zone')) : null,
    }
    setBusy(true)
    setError('')
    try {
      await ok(
        existing
          ? api.PUT('/api/agents/{agent}/wakes/{wake}', {
              params: { path: { agent: agent.id, wake: existing.id } },
              body: wake,
            })
          : api.POST('/api/agents/{agent}/wakes', {
              params: { path: { agent: agent.id } },
              body: { request_id: crypto.randomUUID(), wake },
            }),
      )
      onDone()
    } catch (caught) {
      setError(failure(caught))
      setBusy(false)
    }
  }
  return (
    <form
      className="form wake-form"
      onSubmit={(event) => {
        event.preventDefault()
        void save(event.currentTarget)
      }}
    >
      <Field label="Instructions" hint={`What ${agent.name} should do`}>
        <textarea
          name="instructions"
          rows={3}
          required
          maxLength={4096}
          defaultValue={existing?.instructions ?? ''}
          placeholder="Check whether the deploy finished and tell me how it went."
        />
      </Field>
      <div className="field">
        <span id="wake-when">When</span>
        <div
          className="tabs segmented segmented--wide"
          role="group"
          aria-labelledby="wake-when"
        >
          <button
            type="button"
            aria-pressed={mode === 'once'}
            onClick={() => setMode('once')}
          >
            <AlarmClock aria-hidden="true" /> Once
          </button>
          <button
            type="button"
            aria-pressed={mode === 'repeats'}
            onClick={() => setMode('repeats')}
          >
            <Repeat aria-hidden="true" /> Repeats
          </button>
        </div>
      </div>
      {mode === 'once' ? (
        <Once value={at} onChange={setAt} />
      ) : (
        <>
          <Field label="Cron" hint="Five fields, in the time zone below">
            <input
              required
              maxLength={128}
              placeholder="0 9 * * 1-5"
              value={cron}
              onChange={(event) => setCron(event.target.value)}
            />
          </Field>
          <div
            className="trigger__presets"
            role="group"
            aria-label="Common schedules"
          >
            {SCHEDULES.map(([label, value]) => (
              <button
                key={value}
                type="button"
                className="chip provider-size"
                aria-pressed={cron === value}
                onClick={() => setCron(value)}
              >
                {label}
              </button>
            ))}
          </div>
          <ZoneField existing={existing} />
        </>
      )}
      <Alert>{error}</Alert>
      <div className="actions">
        <button className="btn btn--primary btn--sm" disabled={busy}>
          {busy ? 'Saving…' : existing ? 'Save wake' : 'Set wake'}
        </button>
        <button
          type="button"
          className="btn btn--ghost btn--sm"
          onClick={onDone}
        >
          Cancel
        </button>
      </div>
    </form>
  )
}

const QUICK: [string, () => Date][] = [
  ['In 30 min', () => new Date(Date.now() + 30 * 60_000)],
  ['In 2 hours', () => new Date(Date.now() + 2 * 3_600_000)],
  ['Tomorrow 09:00', () => tomorrowAt(9)],
]

function Once({
  value,
  onChange,
}: {
  value: string
  onChange: (value: string) => void
}) {
  return (
    <>
      <Field label="Date and time" hint="Your device’s time">
        <input
          type="datetime-local"
          required
          min={local(new Date())}
          value={value}
          onChange={(event) => onChange(event.target.value)}
        />
      </Field>
      <div className="trigger__presets" role="group" aria-label="Quick times">
        {QUICK.map(([label, pick]) => (
          <button
            key={label}
            type="button"
            className="chip provider-size"
            onClick={() => onChange(local(pick()))}
          >
            {label}
          </button>
        ))}
      </div>
    </>
  )
}

function soon() {
  const next = new Date(Date.now() + 3_600_000)
  next.setMinutes(Math.ceil(next.getMinutes() / 15) * 15, 0, 0)
  return next
}

function tomorrowAt(hour: number) {
  const next = new Date()
  next.setDate(next.getDate() + 1)
  next.setHours(hour, 0, 0, 0)
  return next
}

function local(date: Date) {
  const shifted = new Date(date.getTime() - date.getTimezoneOffset() * 60_000)
  return shifted.toISOString().slice(0, 16)
}

function moment(value: string, zone: string) {
  return new Intl.DateTimeFormat(undefined, {
    weekday: 'short',
    day: 'numeric',
    month: 'short',
    hour: '2-digit',
    minute: '2-digit',
    hourCycle: 'h23',
    timeZone: zone,
  }).format(new Date(value))
}

export function until(value: string, now = Date.now()) {
  const minutes = Math.round((new Date(value).getTime() - now) / 60_000)
  if (minutes < 1) return 'any moment'
  if (minutes < 60) return `in ${minutes} min`
  const hours = Math.round(minutes / 60)
  if (hours < 48) return `in ${hours} h`
  return `in ${Math.round(hours / 24)} days`
}
