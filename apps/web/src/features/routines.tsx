import { useEffect, useState } from 'react'
import {
  type Agent,
  type Occurrence,
  type Routine,
  api,
  failure,
  ok,
} from '../api/client'

const origins: Record<string, string> = {
  person: 'Run by a person',
  agent: 'Run by the agent',
  schedule: 'Scheduled',
  webhook: 'Webhook',
}

/** Stored prompts and commands the agent runs by hand, on a schedule or by webhook (ADR 0087). */
export function Routines({
  agent,
  onClose,
}: {
  agent: Agent
  onClose: () => void
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
  const rotate = (routine: Routine) =>
    act(async () => {
      const made = await ok(
        api.POST(
          '/api/agents/{agent}/routines/{routine}/secret',
          path(routine),
        ),
      )
      setSecret({ id: routine.id, secret: made.secret })
    })
  const remove = (routine: Routine) =>
    act(() =>
      ok(api.DELETE('/api/agents/{agent}/routines/{routine}', path(routine))),
    )
  return (
    <section>
      <header className="conversation-header">
        <div>
          <h1>{agent.name}: routines</h1>
          <p className="muted">
            Each run is a message in the conversation. A command runs before the
            prompt; without a prompt its result is the answer. Schedules are
            five-field cron in UTC.
          </p>
        </div>
        <div className="actions">
          <button onClick={() => setEditing('new')}>New routine</button>
          <button className="secondary" onClick={onClose}>
            Back to conversation
          </button>
        </div>
      </header>
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
      {error && <p role="alert">{error}</p>}
      {notice && <p role="status">{notice}</p>}
      {routines.length === 0 && <p className="muted">No routines yet.</p>}
      <ul className="memory-list">
        {routines.map((routine) => (
          <li key={routine.id} className="memory">
            <p>
              <strong>{routine.name}</strong>
              {!routine.enabled && ' (paused)'}
            </p>
            {routine.prompt && <p>{routine.prompt}</p>}
            {routine.command && (
              <p>
                <code>{routine.command}</code>
              </p>
            )}
            <p className="muted">{when(routine)}</p>
            {routine.webhook && (
              <p className="muted">
                Webhook: POST <code>{hookUrl(routine)}</code> with{' '}
                <code>Authorization: Bearer</code> and the secret
              </p>
            )}
            {secret?.id === routine.id && (
              <p role="status">
                Secret, shown only now: <code>{secret.secret}</code>
              </p>
            )}
            <div className="memory-meta">
              <button
                className="text-button"
                onClick={() => void runNow(routine)}
              >
                Run now
              </button>
              <button
                className="text-button"
                onClick={() => setEditing(routine)}
              >
                Edit
              </button>
              <button
                className="text-button"
                onClick={() => void rotate(routine)}
              >
                {routine.webhook ? 'New webhook secret' : 'Add webhook'}
              </button>
              <button
                className="text-button"
                onClick={() => void remove(routine)}
              >
                Delete
              </button>
            </div>
            <History agent={agent.id} routine={routine.id} />
          </li>
        ))}
      </ul>
    </section>
  )
}

function when(routine: Routine) {
  const managed =
    routine.managed_by === 'agent'
      ? 'Managed by the agent'
      : 'Managed by a person'
  const schedule = routine.schedule
    ? `${routine.schedule} UTC, next ${routine.next_at ? new Date(routine.next_at).toLocaleString() : 'never'}`
    : 'On demand'
  const last = routine.last_run_at
    ? `, last run ${new Date(routine.last_run_at).toLocaleString()}`
    : ''
  const coalesce = routine.coalesce ? ', skips a run while one waits' : ''
  return `${managed} · ${schedule}${last}${coalesce}`
}

function hookUrl(routine: Routine) {
  return `${window.location.origin}/api/hooks/${routine.id}`
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
    <details onToggle={(event) => event.currentTarget.open && load()}>
      <summary>History</summary>
      {error && <p role="alert">{error}</p>}
      {runs?.length === 0 && <p className="muted">No runs yet.</p>}
      <ul className="memory-list">
        {runs?.map((run) => (
          <li key={run.turn} className="memory-meta">
            <span>{new Date(run.created_at).toLocaleString()}</span>
            <span className="muted">
              {origins[run.origin] ?? run.origin} · {run.status}
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
  agent: Agent
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
      onSubmit={(event) => {
        event.preventDefault()
        void save(event.currentTarget)
      }}
    >
      <Fields existing={existing} />
      {!existing && (
        <label>
          <input type="checkbox" name="webhook" /> Make a webhook
        </label>
      )}
      {error && <p role="alert">{error}</p>}
      <div className="actions">
        <button disabled={busy}>{busy ? 'Saving…' : 'Save'}</button>
        <button type="button" className="secondary" onClick={() => onDone()}>
          Cancel
        </button>
      </div>
    </form>
  )
}

function Fields({ existing }: { existing: Routine | undefined }) {
  return (
    <>
      <label>
        Name
        <input
          name="name"
          required
          maxLength={128}
          defaultValue={existing?.name ?? ''}
        />
      </label>
      <label>
        Prompt
        <textarea
          name="prompt"
          rows={3}
          maxLength={8192}
          defaultValue={existing?.prompt ?? ''}
        />
      </label>
      <label>
        Command
        <input
          name="command"
          maxLength={4096}
          placeholder="Runs with bash before the prompt"
          defaultValue={existing?.command ?? ''}
        />
      </label>
      <label>
        Schedule
        <input
          name="schedule"
          maxLength={128}
          placeholder="0 9 * * 1-5, empty to run only on demand"
          defaultValue={existing?.schedule ?? ''}
        />
      </label>
      <label>
        <input
          type="checkbox"
          name="coalesce"
          defaultChecked={existing?.coalesce ?? true}
        />{' '}
        Skip a run while another is still waiting
      </label>
      <label>
        <input
          type="checkbox"
          name="enabled"
          defaultChecked={existing?.enabled ?? true}
        />{' '}
        Enabled; a paused routine runs only by hand
      </label>
    </>
  )
}

type Secret = { id: string; secret: string }

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
