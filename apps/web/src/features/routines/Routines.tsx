import { Link } from '@tanstack/react-router'
import { Play, Webhook } from 'lucide-react'
import { useEffect, useState } from 'react'
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
  CopyButton,
  Empty,
  Field,
  Status,
  when,
} from '../../components/primitives'
import './routines.css'

const ORIGINS: Record<string, string> = {
  person: 'Run by a person',
  agent: 'Run by the agent',
  schedule: 'Scheduled',
  webhook: 'Webhook',
}
type Secret = { id: string; secret: string }
type HookDelivery = components['schemas']['HookDelivery']

/** Stored prompts and commands the agent runs by hand, on a schedule or by webhook (docs/adr/agents.md#routines). */
export function Routines({
  agent,
  editable,
}: {
  agent: AgentSummary
  editable: boolean
}) {
  const [routines, setRoutines] = useState<Routine[]>([])
  const [editing, setEditing] = useState<Routine | 'new' | null>(null)
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
  const rotate = (routine: Routine) => {
    // A live secret stops working the moment it is replaced.
    if (
      routine.webhook &&
      !window.confirm(
        `Replace the webhook secret of ${routine.name}? Whatever sends to it stops working until it has the new one.`,
      )
    )
      return
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
  const removeHook = (routine: Routine) => {
    if (
      !window.confirm(
        `Remove the webhook of ${routine.name}? Its URL stops working; the routine stays.`,
      )
    )
      return
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
          A routine is a stored prompt or command. Run it by hand, on a schedule
          or from a webhook; each run is a message in the conversation.
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
              <p className="small faint">{describe(routine)}</p>
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

function describe(routine: Routine) {
  const managed =
    routine.managed_by === 'agent'
      ? 'Managed by the agent'
      : 'Managed by a person'
  const schedule = routine.schedule
    ? `next ${routine.next_at ? when(routine.next_at) : 'never'}`
    : 'On demand'
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

/** A webhook is one way to trigger its routine, so it lives on the routine: where to send, how to
 *  authenticate, and what arrived. */
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

/** What reached the webhook in the last 30 days, loaded when opened. A delivery with a wrong
 *  secret never reached the routine: the caller gets the 401 and it is not listed. */
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

/** The routine's latest runs, loaded when opened. */
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

/** Creates or replaces a routine; a new one with a webhook hands back its secret once. */
function RoutineForm({
  agent,
  existing,
  onDone,
}: {
  agent: AgentSummary
  existing: Routine | undefined
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
      <Field
        label="Schedule"
        hint="Five-field cron, UTC; empty runs only on demand"
      >
        <input
          name="schedule"
          maxLength={128}
          placeholder="0 9 * * 1-5"
          defaultValue={existing?.schedule ?? ''}
        />
      </Field>
      <label className="check">
        <input
          type="checkbox"
          name="coalesce"
          defaultChecked={existing?.coalesce ?? true}
        />
        Skip a run while another is still waiting
      </label>
      <label className="check">
        <input
          type="checkbox"
          name="enabled"
          defaultChecked={existing?.enabled ?? true}
        />
        Enabled; a paused routine runs only by hand
      </label>
      {!existing && (
        <label className="check">
          <input type="checkbox" name="webhook" /> Give it a webhook, so another
          system can run it
        </label>
      )}
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

/** Saves the form; a new routine's webhook secret, if it made one. */
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
    coalesce: data.has('coalesce'),
    enabled: data.has('enabled'),
  }
}

function listRoutines(agent: string) {
  return ok(
    api.GET('/api/agents/{agent}/routines', { params: { path: { agent } } }),
  )
}
