import { Link } from '@tanstack/react-router'
import { useQuery } from '@tanstack/react-query'
import { CalendarClock, Play, Webhook, Zap } from 'lucide-react'
import { useEffect, useMemo, useState } from 'react'
import {
  api,
  failure,
  ok,
  type Occurrence,
  type Routine,
} from '../../api/client'
import type { components } from '../../api/schema'
import type { AgentSummary } from '../../api/types'
import {
  Alert,
  confirmAction,
  CopyButton,
  Empty,
  Field,
  Status,
  when,
} from '../../components/primitives'
import { scheduleWords } from '../calendar/model'
import { useProfile } from '../shell/AboutYou'
import './routines.css'

const SCHEDULES = [
  ['Weekdays at 09:00', '0 9 * * 1-5'],
  ['Every day at 09:00', '0 9 * * *'],
  ['Every hour', '0 * * * *'],
  ['Mondays at 09:00', '0 9 * * 1'],
] as const

const ORIGINS: Record<string, string> = {
  person: 'Run by a person',
  agent: 'Run by the agent',
  schedule: 'Scheduled',
  webhook: 'Webhook',
  event: 'Connection event',
}
type Secret = { id: string; secret: string }
type HookDelivery = components['schemas']['HookDelivery']

export function Routines({
  agent,
  editable,
}: {
  agent: AgentSummary
  editable: boolean
}) {
  const [routines, setRoutines] = useState<Routine[]>([])
  const [editing, setEditing] = useState<Routine | 'new' | null>(null)
  const zone = useViewerZone()
  const [secret, setSecret] = useState<Secret | null>(null)
  const [error, setError] = useState('')
  const [notice, setNotice] = useState('')
  async function load() {
    setRoutines(await listRoutines(agent.id))
  }
  useEffect(() => {
    void listRoutines(agent.id)
      .then(setRoutines)
      .catch((error) => setError(failure(error)))
  }, [agent.id])
  async function act(action: () => Promise<unknown>) {
    setError('')
    setNotice('')
    try {
      await action()
      await load()
    } catch (error) {
      setError(failure(error))
    }
  }
  const path = (routine: Routine) => ({
    params: { path: { agent: agent.id, routine: routine.id } },
  })
  const runNow = (routine: Routine) =>
    act(async () => {
      const ran = await ok(
        api.POST('/api/agents/{agent}/routines/{routine}/runs', {
          ...path(routine),
          body: { request_id: crypto.randomUUID() },
        }),
      )
      setNotice(
        ran.turn
          ? `${routine.name} is queued in the conversation.`
          : `A run of ${routine.name} is already waiting; nothing new was queued.`,
      )
    })
  const rotate = async (routine: Routine) => {
    // A live secret stops working the moment it is replaced.
    const go =
      !routine.webhook ||
      (await confirmAction({
        title: `Replace the webhook secret of ${routine.name}?`,
        body: 'Whatever sends to it stops working until it has the new one.',
        action: 'Replace',
      }))
    if (!go) return
    void act(async () => {
      const made = await ok(
        api.POST(
          '/api/agents/{agent}/routines/{routine}/secret',
          path(routine),
        ),
      )
      setSecret({ id: routine.id, secret: made.secret })
    })
  }
  const removeHook = async (routine: Routine) => {
    const yes = await confirmAction({
      title: `Remove the webhook of ${routine.name}?`,
      body: 'Its URL stops working; the routine stays.',
      action: 'Remove',
    })
    if (!yes) return
    void act(async () => {
      await ok(
        api.DELETE(
          '/api/agents/{agent}/routines/{routine}/secret',
          path(routine),
        ),
      )
      setSecret(null)
    })
  }
  const remove = (routine: Routine) =>
    act(() =>
      ok(api.DELETE('/api/agents/{agent}/routines/{routine}', path(routine))),
    )
  return (
    <>
      <section className="panel__section">
        <h2>Routines</h2>
        <p className="panel__lead">
          A routine is a stored prompt or command, run by hand, on a schedule or
          from a webhook. Each run is a message in the conversation.
        </p>
        {editable && !editing && (
          <div className="actions">
            <button
              type="button"
              className="btn btn--primary btn--sm"
              onClick={() => setEditing('new')}
            >
              New routine
            </button>
          </div>
        )}
        {editing && (
          <RoutineForm
            agent={agent}
            existing={editing === 'new' ? undefined : editing}
            onDone={(made) => {
              setEditing(null)
              if (made) setSecret(made)
              void load().catch((error) => setError(failure(error)))
            }}
          />
        )}
        <Alert>{error}</Alert>
        <Status>{notice}</Status>
      </section>
      <section className="panel__section">
        {routines.length === 0 && (
          <Empty title="No routines yet.">
            Create one to run a prompt or a command by hand, on a schedule, or
            whenever another system calls its webhook.
          </Empty>
        )}
        <ul className="list">
          {routines.map((routine) => (
            <li key={routine.id}>
              <div className="title">
                {routine.name}
                {!routine.enabled && <span className="chip">paused</span>}
                {routine.schedule && (
                  <span className="chip chip--sage">{routine.schedule}</span>
                )}
              </div>
              {routine.prompt && (
                <p className="small" style={{ whiteSpace: 'pre-wrap' }}>
                  {routine.prompt}
                </p>
              )}
              {routine.command && (
                <p className="small">
                  <code>{routine.command}</code>
                </p>
              )}
              <p className="small faint">{describe(routine, zone)}</p>
              {routine.webhook && (
                <WebhookDetails
                  agent={agent.id}
                  routine={routine}
                  secret={secret?.id === routine.id ? secret.secret : null}
                  editable={editable}
                  onRotate={() => rotate(routine)}
                  onRemove={() => removeHook(routine)}
                />
              )}
              {editable && (
                <div className="actions" style={{ marginTop: 6 }}>
                  <button
                    type="button"
                    className="btn btn--sm"
                    onClick={() => void runNow(routine)}
                  >
                    <Play /> Run now
                  </button>
                  <button
                    type="button"
                    className="link-btn small"
                    onClick={() => setEditing(routine)}
                  >
                    Edit
                  </button>
                  {!routine.webhook && (
                    <button
                      type="button"
                      className="link-btn small"
                      onClick={() => rotate(routine)}
                    >
                      Add webhook
                    </button>
                  )}
                  <button
                    type="button"
                    className="link-btn small"
                    onClick={() => void remove(routine)}
                  >
                    Delete
                  </button>
                </div>
              )}
              <History agent={agent.id} routine={routine.id} />
            </li>
          ))}
        </ul>
      </section>
    </>
  )
}

/** The zone the person reads times in: their profile's, else this device's. */
function useViewerZone() {
  const profile = useProfile()
  return (
    profile.data?.time_zone || Intl.DateTimeFormat().resolvedOptions().timeZone
  )
}

function describe(routine: Routine, zone: string) {
  const managed =
    routine.managed_by === 'agent'
      ? 'Managed by the agent'
      : 'Managed by a person'
  const schedule = routine.schedule
    ? scheduleWords(routine.schedule, routine.time_zone, routine.next_at, zone)
    : routine.trigger
      ? `Runs on ${routine.trigger.connection}'s ${routine.trigger.event}`
      : routine.webhook
        ? 'Runs from its webhook or by hand'
        : 'Runs by hand'
  const last = routine.last_run_at
    ? `, last run ${when(routine.last_run_at)}`
    : ''
  const coalesce = routine.coalesce ? ', skips a run while one waits' : ''
  return `${managed} · ${schedule}${last}${coalesce}`
}

function hookUrl(routine: Routine) {
  return `${window.location.origin}/api/hooks/${routine.id}`
}

const OUTCOMES: Record<string, string> = {
  accepted: 'Started a run',
  coalesced: 'Joined the waiting run',
  replayed: 'Repeat of an earlier delivery',
  paused: 'Refused: the routine is paused',
}

function WebhookDetails({
  agent,
  routine,
  secret,
  editable,
  onRotate,
  onRemove,
}: {
  agent: string
  routine: Routine
  secret: string | null
  editable: boolean
  onRotate: () => void
  onRemove: () => void
}) {
  const url = hookUrl(routine)
  const example = `curl -X POST ${url} \\
  -H "Authorization: Bearer ${secret ?? '$WEBHOOK_SECRET'}" \\
  -H "Idempotency-Key: $(uuidgen)" \\
  -d '{"event":"test"}'`
  return (
    <div className="hook">
      <div className="hook__title">
        <Webhook aria-hidden="true" />
        <strong>Webhook</strong>
        <span className="faint small">
          {!routine.enabled
            ? 'Paused; deliveries are refused'
            : routine.last_delivery_at
              ? `Last delivery ${when(routine.last_delivery_at)}`
              : 'No deliveries yet'}
        </span>
      </div>
      <div className="hook__line">
        <code>{url}</code>
        <CopyButton text={url} label="Copy webhook URL" />
      </div>
      {secret && (
        <div className="hook__secret" role="status">
          <p className="small">
            Copy this secret now. It is shown only once and stored only as a
            digest.
          </p>
          <div className="hook__line">
            <code>{secret}</code>
            <CopyButton text={secret} label="Copy webhook secret" />
          </div>
        </div>
      )}
      <details className="disclosure">
        <summary>How to call it</summary>
        <div className="hook__how small">
          <p>
            Send a POST with the secret as a bearer token. The body, up to 8
            KiB, reaches the agent as untrusted data it can read but never takes
            instructions from. An Idempotency-Key makes a retry safe for a day.
            Each routine takes 60 deliveries a minute.
          </p>
          <div className="hook__line hook__line--block">
            <pre>
              <code>{example}</code>
            </pre>
            <CopyButton text={example} label="Copy example command" />
          </div>
        </div>
      </details>
      <Deliveries agent={agent} routine={routine.id} />
      {editable && (
        <div className="actions">
          <button type="button" className="link-btn small" onClick={onRotate}>
            New secret
          </button>
          <button type="button" className="link-btn small" onClick={onRemove}>
            Remove webhook
          </button>
        </div>
      )}
    </div>
  )
}

/** A delivery with a wrong secret never reached the routine: the caller gets the 401 and it is not listed. */
function Deliveries({ agent, routine }: { agent: string; routine: string }) {
  const [rows, setRows] = useState<HookDelivery[] | null>(null)
  const [error, setError] = useState('')
  function load() {
    setError('')
    void ok(
      api.GET('/api/agents/{agent}/routines/{routine}/deliveries', {
        params: { path: { agent, routine } },
      }),
    )
      .then(setRows)
      .catch((error) => setError(failure(error)))
  }
  return (
    <details
      className="disclosure"
      onToggle={(event) => event.currentTarget.open && load()}
    >
      <summary>Deliveries</summary>
      <Alert>{error}</Alert>
      {rows?.length === 0 && (
        <p className="small faint">Nothing has reached this webhook yet.</p>
      )}
      <ul className="hook__deliveries">
        {rows?.map((row, index) => (
          <li key={`${row.received_at}:${index}`} className="small">
            <span className="hook__when">{when(row.received_at)}</span>
            <span>
              {OUTCOMES[row.outcome] ?? row.outcome}
              {row.turn && row.status && (
                <>
                  {' · '}
                  <Link
                    to="/agents/$agentId/thread/$turnId"
                    params={{ agentId: agent, turnId: row.turn }}
                  >
                    {row.status}
                  </Link>
                </>
              )}
            </span>
            <span className="faint hook__bytes">{row.bytes} B</span>
          </li>
        ))}
      </ul>
    </details>
  )
}

function History({ agent, routine }: { agent: string; routine: string }) {
  const [runs, setRuns] = useState<Occurrence[] | null>(null)
  const [error, setError] = useState('')
  function load() {
    setError('')
    void ok(
      api.GET('/api/agents/{agent}/routines/{routine}/runs', {
        params: { path: { agent, routine } },
      }),
    )
      .then(setRuns)
      .catch((error) => setError(failure(error)))
  }
  return (
    <details
      className="disclosure"
      style={{ marginTop: 6 }}
      onToggle={(event) => event.currentTarget.open && load()}
    >
      <summary>History</summary>
      <Alert>{error}</Alert>
      {runs?.length === 0 && <p className="small faint">No runs yet.</p>}
      <ul className="list">
        {runs?.map((run) => (
          <li key={run.turn} className="small">
            {when(run.created_at)}{' '}
            <span className="faint">
              · {ORIGINS[run.origin] ?? run.origin} · {run.status}
              {run.error && ` (${run.error})`}
            </span>
          </li>
        ))}
      </ul>
    </details>
  )
}

export function RoutineForm({
  agent,
  existing,
  schedule,
  onDone,
}: {
  agent: AgentSummary
  existing: Routine | undefined
  /** A new routine's schedule, such as the calendar slot it was made from. */
  schedule?: string
  onDone: (secret?: Secret) => void
}) {
  const [error, setError] = useState('')
  const [busy, setBusy] = useState(false)
  async function save(form: HTMLFormElement) {
    const data = new FormData(form)
    const routine = routineBody(data)
    if (!routine.prompt && !routine.command) {
      setError('Give the routine a prompt, a command or both.')
      return
    }
    setBusy(true)
    setError('')
    try {
      onDone(await saveRoutine(agent.id, existing, data, routine))
    } catch (error) {
      setError(failure(error))
    } finally {
      setBusy(false)
    }
  }
  return (
    <form
      className="form"
      onSubmit={(event) => {
        event.preventDefault()
        void save(event.currentTarget)
      }}
    >
      <Field label="Name">
        <input
          name="name"
          required
          maxLength={128}
          defaultValue={existing?.name ?? ''}
        />
      </Field>
      <Field label="Prompt">
        <textarea
          name="prompt"
          rows={3}
          maxLength={8192}
          defaultValue={existing?.prompt ?? ''}
        />
      </Field>
      <Field label="Command" hint="Runs with bash before the prompt">
        <input
          name="command"
          maxLength={4096}
          defaultValue={existing?.command ?? ''}
        />
      </Field>
      <Triggers agent={agent.id} existing={existing} schedule={schedule} />
      <label className="check">
        <input
          type="checkbox"
          name="enabled"
          defaultChecked={existing?.enabled ?? true}
        />
        Active
        <span className="small faint">
          While paused, its schedule and webhook do nothing
        </span>
      </label>
      <label className="check">
        <input
          type="checkbox"
          name="coalesce"
          defaultChecked={existing?.coalesce ?? true}
        />
        Skip a run while another is still waiting
      </label>
      <Alert>{error}</Alert>
      <div className="actions">
        <button className="btn btn--primary btn--sm" disabled={busy}>
          {busy ? 'Saving…' : 'Save'}
        </button>
        <button
          type="button"
          className="btn btn--ghost btn--sm"
          onClick={() => onDone()}
        >
          Cancel
        </button>
      </div>
    </form>
  )
}

function Triggers({
  agent,
  existing,
  schedule,
}: {
  agent: string
  existing: Routine | undefined
  schedule: string | undefined
}) {
  const [scheduled, setScheduled] = useState(
    Boolean(existing?.schedule ?? schedule),
  )
  const [cron, setCron] = useState<string>(
    existing?.schedule ?? schedule ?? SCHEDULES[0][1],
  )
  return (
    <fieldset className="triggers">
      <legend>When it runs</legend>
      <p className="small faint">
        You can always run it by hand. Add a schedule, a webhook, a connected
        account’s events, any of them or none.
      </p>
      <label className="trigger">
        <input
          type="checkbox"
          checked={scheduled}
          onChange={(event) => setScheduled(event.target.checked)}
        />
        <CalendarClock aria-hidden="true" />
        <span>
          On a schedule
          <span className="small faint">Repeats at the times you set</span>
        </span>
      </label>
      {scheduled && (
        <div className="trigger__more">
          <Field label="Cron" hint="Five fields, in the time zone below">
            <input
              name="schedule"
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
        </div>
      )}
      {existing?.webhook ? (
        <p className="trigger trigger--static">
          <Webhook aria-hidden="true" />
          <span>
            From its webhook
            <span className="small faint">Manage it on the routine below</span>
          </span>
        </p>
      ) : existing ? (
        <p className="small faint trigger--hint">
          Add a webhook with the button on the routine.
        </p>
      ) : (
        <label className="trigger">
          <input type="checkbox" name="webhook" />
          <Webhook aria-hidden="true" />
          <span>
            From a webhook
            <span className="small faint">
              Another system runs it by calling a private URL
            </span>
          </span>
        </label>
      )}
      <EventTrigger agent={agent} existing={existing} />
    </fieldset>
  )
}

/** Defaults to the zone the person reads times in. */
function ZoneField({ existing }: { existing: Routine | undefined }) {
  const viewer = useViewerZone()
  const zones = useMemo(() => Intl.supportedValuesOf('timeZone'), [])
  // An unscheduled routine holds the column default, UTC, which nobody chose.
  const [chosen, setChosen] = useState(
    existing?.schedule ? existing.time_zone : undefined,
  )
  const zone = chosen ?? viewer
  return (
    <Field label="Time zone">
      <select
        name="time_zone"
        value={zone}
        onChange={(event) => setChosen(event.target.value)}
      >
        {!zones.includes(zone) && <option value={zone}>{zone}</option>}
        {zones.map((option) => (
          <option key={option} value={option}>
            {option.replaceAll('_', ' ')}
          </option>
        ))}
      </select>
    </Field>
  )
}

/** Connections this agent may use and the events they report; hidden when it may use none. */
function EventTrigger({
  agent,
  existing,
}: {
  agent: string
  existing: Routine | undefined
}) {
  const connections = useQuery({
    queryKey: ['agent-connections', agent],
    queryFn: () =>
      ok(
        api.GET('/api/agents/{id}/connections', {
          params: { path: { id: agent } },
        }),
      ),
    retry: false,
  })
  const current = existing?.trigger
    ? `${existing.trigger.connection} ${existing.trigger.event}`
    : ''
  const [chosen, setChosen] = useState(current)
  const options = (connections.data ?? [])
    .filter((connection) => connection.enabled)
    .flatMap((connection) =>
      connection.events.map((event) => ({
        value: `${connection.slug} ${event}`,
        label: `${connection.slug}: ${event}`,
      })),
    )
  if (options.length === 0 && !current) return null
  return (
    <>
      <label className="trigger">
        <input
          type="checkbox"
          checked={chosen !== ''}
          onChange={(event) =>
            setChosen(event.target.checked ? (options[0]?.value ?? '') : '')
          }
        />
        <Zap aria-hidden="true" />
        <span>
          When a connected account reports an event
          <span className="small faint">
            Such as new mail; the event arrives in the run’s message
          </span>
        </span>
      </label>
      {chosen && (
        <div className="trigger__more">
          <Field label="Event">
            <select
              name="trigger"
              value={chosen}
              onChange={(event) => setChosen(event.target.value)}
            >
              {!options.some((option) => option.value === current) &&
                current && <option value={current}>{current}</option>}
              {options.map((option) => (
                <option key={option.value} value={option.value}>
                  {option.label}
                </option>
              ))}
            </select>
          </Field>
        </div>
      )}
    </>
  )
}

async function saveRoutine(
  agent: string,
  existing: Routine | undefined,
  data: FormData,
  routine: ReturnType<typeof routineBody>,
): Promise<Secret | undefined> {
  if (existing) {
    await ok(
      api.PUT('/api/agents/{agent}/routines/{routine}', {
        params: { path: { agent, routine: existing.id } },
        body: { expected_revision: existing.revision, routine },
      }),
    )
    return undefined
  }
  const made = await ok(
    api.POST('/api/agents/{agent}/routines', {
      params: { path: { agent } },
      body: {
        request_id: crypto.randomUUID(),
        webhook: data.has('webhook'),
        routine,
      },
    }),
  )
  return made.secret ? { id: made.routine.id, secret: made.secret } : undefined
}

function routineBody(data: FormData) {
  const text = (name: string) => String(data.get(name) ?? '').trim() || null
  return {
    name: text('name') ?? '',
    prompt: text('prompt'),
    command: text('command'),
    schedule: text('schedule'),
    time_zone: text('time_zone'),
    coalesce: data.has('coalesce'),
    enabled: data.has('enabled'),
    trigger: trigger(text('trigger')),
  }
}

function trigger(chosen: string | null) {
  const [connection, event] = chosen?.split(' ') ?? []
  return connection && event ? { connection, event } : null
}

function listRoutines(agent: string) {
  return ok(
    api.GET('/api/agents/{agent}/routines', { params: { path: { agent } } }),
  )
}
