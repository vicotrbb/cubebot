import { useQueryClient } from '@tanstack/react-query'
import { useState } from 'react'
import { Link } from '@tanstack/react-router'
import { api, ApiError, failure, ok } from '../../api/client'
import {
  admin,
  type DefinitionRevision,
  type LifecycleAction,
} from '../../api/admin'
import type { AgentSummary } from '../../api/types'
import { keys, useInstallation } from '../../app/queries'
import { moodOf, type Avatar } from '../../components/Cube'
import { Alert, Field, Status, Switch } from '../../components/primitives'
import { AgentSpeechSection } from '../call/SpeechSettings'
import { AvatarBuilder } from './AvatarBuilder'
import './about.css'

/** Everything a manager changes about an agent: identity, instructions, behaviour, lifecycle. */
export function About({ agent }: { agent: AgentSummary }) {
  const client = useQueryClient()
  const refresh = () => client.invalidateQueries({ queryKey: keys.agents })
  return (
    <>
      <Identity agent={agent} onSaved={refresh} />
      <Instructions agent={agent} onSaved={refresh} />
      <Behaviour agent={agent} onSaved={refresh} />
      <Capabilities agent={agent} onSaved={refresh} />
      <AgentSpeechSection agent={agent} />
      <Lifecycle agent={agent} onChanged={refresh} />
      <Tools agentId={agent.id} />
    </>
  )
}

/** The definition fields the update route requires; a section edits its own part. */
function definition(agent: AgentSummary) {
  return {
    expected_revision: agent.revision,
    name: agent.name,
    instructions: agent.instructions ?? '',
    idle_policy: agent.idle_policy,
    idle_delay_seconds: agent.idle_delay_seconds,
    dreaming_model: agent.dreaming_model ?? null,
    task_lanes: agent.task_lanes,
  }
}

function useSave(onSaved: () => Promise<unknown>) {
  const [error, setError] = useState('')
  const [notice, setNotice] = useState('')
  const [busy, setBusy] = useState(false)
  async function save(action: () => Promise<unknown>, done = 'Saved.') {
    setBusy(true)
    setError('')
    setNotice('')
    try {
      await action()
      await onSaved()
      setNotice(done)
    } catch (error) {
      setError(failure(error))
    } finally {
      setBusy(false)
    }
  }
  return { save, error, notice, busy }
}

function Identity({
  agent,
  onSaved,
}: {
  agent: AgentSummary
  onSaved: () => Promise<unknown>
}) {
  const { save, error, notice, busy } = useSave(onSaved)
  const [name, setName] = useState(agent.name)
  const [avatar, setAvatar] = useState<Avatar>(agent.avatar)
  const changed =
    name !== agent.name ||
    JSON.stringify(avatar) !== JSON.stringify(agent.avatar)
  return (
    <section className="panel__section">
      <h2>About</h2>
      <form
        className="form"
        onSubmit={(event) => {
          event.preventDefault()
          void save(() =>
            admin.setAvatar(agent.id, { ...definition(agent), name }, avatar),
          )
        }}
      >
        <AvatarBuilder
          value={avatar}
          mood={moodOf(agent)}
          onChange={setAvatar}
        />
        <Field label="Name">
          <input
            value={name}
            onChange={(event) => setName(event.target.value)}
            required
            maxLength={255}
          />
        </Field>
        <Wallpaper agent={agent} />
        <Alert>{error}</Alert>
        <Status>{notice}</Status>
        <div className="actions">
          <button
            className="btn btn--primary btn--sm"
            disabled={busy || !changed}
          >
            {busy ? 'Saving…' : 'Save'}
          </button>
        </div>
      </form>
    </section>
  )
}

function Wallpaper({ agent }: { agent: AgentSummary }) {
  const [error, setError] = useState('')
  const [notice, setNotice] = useState('')
  const [busy, setBusy] = useState(false)
  // A JPEG or PNG of at most 4 MiB, or `null` to reset to the default.
  async function set(file: File | null) {
    setError('')
    setNotice('')
    if (file && file.size > 4 * 1024 * 1024)
      return setError('Choose an image of at most 4 MiB.')
    setBusy(true)
    try {
      const response = await fetch(`/api/agents/${agent.id}/wallpaper`, {
        method: file ? 'PUT' : 'DELETE',
        body: file,
        credentials: 'same-origin',
      })
      if (!response.ok) throw new ApiError(response.status)
      setNotice(file ? 'Wallpaper set.' : 'Wallpaper reset to the default.')
    } catch (error) {
      setError(failure(error))
    } finally {
      setBusy(false)
    }
  }
  return (
    <Field label="Desktop wallpaper" hint="JPEG or PNG, up to 4 MiB">
      <div className="actions">
        <label className="btn btn--sm">
          Choose image
          <input
            type="file"
            accept="image/jpeg,image/png"
            disabled={busy}
            className="sr-only"
            onChange={(event) => {
              const file = event.currentTarget.files?.[0]
              event.currentTarget.value = ''
              if (file) void set(file)
            }}
          />
        </label>
        <button
          type="button"
          className="link-btn small"
          disabled={busy}
          onClick={() => void set(null)}
        >
          Reset
        </button>
      </div>
      <Alert>{error}</Alert>
      <Status>{notice}</Status>
    </Field>
  )
}

function Instructions({
  agent,
  onSaved,
}: {
  agent: AgentSummary
  onSaved: () => Promise<unknown>
}) {
  const { save, error, notice, busy } = useSave(onSaved)
  const [text, setText] = useState(agent.instructions ?? '')
  const [revisions, setRevisions] = useState<DefinitionRevision[] | null>(null)
  const [viewing, setViewing] = useState<DefinitionRevision | null>(null)
  return (
    <section className="panel__section">
      <h2>Instructions</h2>
      <p className="panel__lead">
        The agent reads these at the start of every turn. Revision{' '}
        {agent.instructions_revision}.
      </p>
      <form
        className="form"
        onSubmit={(event) => {
          event.preventDefault()
          void save(() =>
            ok(
              api.POST('/api/agents/{id}/definition', {
                params: { path: { id: agent.id } },
                body: {
                  request_id: crypto.randomUUID(),
                  ...definition(agent),
                  instructions: text,
                },
              }),
            ),
          )
        }}
      >
        <Field label="Instructions">
          <textarea
            value={text}
            rows={10}
            maxLength={65536}
            onChange={(event) => setText(event.target.value)}
          />
        </Field>
        <Alert>{error}</Alert>
        <Status>{notice}</Status>
        <div className="actions">
          <button
            className="btn btn--primary btn--sm"
            disabled={busy || text === (agent.instructions ?? '')}
          >
            {busy ? 'Saving…' : 'Save instructions'}
          </button>
          <button
            type="button"
            className="link-btn small"
            onClick={() =>
              void admin
                .agentRevisions(agent.id)
                .then((list) => setRevisions([...list].reverse()))
                .catch(() => setRevisions([]))
            }
          >
            History
          </button>
        </div>
      </form>
      {revisions && (
        <ul className="list">
          {revisions.length === 0 && (
            <li className="small faint">No earlier revisions.</li>
          )}
          {revisions.map((revision) => (
            <li key={revision.revision}>
              <div className="title small">
                Revision {revision.revision}
                <span className="faint" style={{ fontWeight: 400 }}>
                  · instructions rev {revision.instructions_revision} ·{' '}
                  {revision.name}
                </span>
                <button
                  type="button"
                  className="link-btn small"
                  onClick={() =>
                    setViewing(viewing === revision ? null : revision)
                  }
                >
                  {viewing === revision ? 'Hide' : 'Show'}
                </button>
              </div>
              {viewing === revision && (
                <pre className="small">{revision.instructions}</pre>
              )}
            </li>
          ))}
        </ul>
      )}
    </section>
  )
}

function Behaviour({
  agent,
  onSaved,
}: {
  agent: AgentSummary
  onSaved: () => Promise<unknown>
}) {
  const { save, error, notice, busy } = useSave(onSaved)
  return (
    <section className="panel__section">
      <h2>Behaviour</h2>
      <form
        className="form"
        onSubmit={(event) => {
          event.preventDefault()
          const data = new FormData(event.currentTarget)
          void save(() =>
            ok(
              api.POST('/api/agents/{id}/definition', {
                params: { path: { id: agent.id } },
                body: {
                  request_id: crypto.randomUUID(),
                  ...definition(agent),
                  idle_delay_seconds: Number(data.get('idle_minutes')) * 60,
                  dreaming_model:
                    String(data.get('dreaming_model')).trim() || null,
                  task_lanes: Number(data.get('task_lanes')),
                },
              }),
            ),
          )
        }}
      >
        <div className="row">
          <Field
            label="Idle minutes before sleeping"
            hint="It dreams while it sleeps"
          >
            <input
              name="idle_minutes"
              type="number"
              required
              min={1}
              max={1440}
              defaultValue={Math.round(agent.idle_delay_seconds / 60)}
            />
          </Field>
        </div>
        <div className="row">
          <Field label="Delegated tasks at once" hint="1 to 4 task lanes">
            <input
              name="task_lanes"
              type="number"
              required
              min={1}
              max={4}
              defaultValue={agent.task_lanes}
            />
          </Field>
          <Field label="Dreaming model" hint="Empty uses the provider model">
            <input
              name="dreaming_model"
              maxLength={200}
              defaultValue={agent.dreaming_model ?? ''}
            />
          </Field>
        </div>
        <Alert>{error}</Alert>
        <Status>{notice}</Status>
        <div className="actions">
          <button className="btn btn--primary btn--sm" disabled={busy}>
            {busy ? 'Saving…' : 'Save behaviour'}
          </button>
        </div>
      </form>
    </section>
  )
}

/** What the agent may do beyond its own conversation; each switch applies at once. */
function Capabilities({
  agent,
  onSaved,
}: {
  agent: AgentSummary
  onSaved: () => Promise<unknown>
}) {
  const { save, error, busy } = useSave(onSaved)
  return (
    <section className="panel__section">
      <h2>Capabilities</h2>
      <div className="capability">
        <div>
          <p className="capability__name" id={`broadcast-${agent.id}`}>
            Broadcast to every agent
          </p>
          <p className="faint small">
            Lets {agent.name} post in the Broadcast channel, which reaches every
            agent at once. A broadcast never answers another broadcast.
          </p>
        </div>
        <Switch
          checked={agent.broadcast}
          label={`${agent.name} may broadcast to every agent`}
          disabled={busy || agent.desired_state === 'retired'}
          onChange={(broadcast) =>
            void save(() =>
              ok(
                api.POST('/api/agents/{id}/definition', {
                  params: { path: { id: agent.id } },
                  body: {
                    request_id: crypto.randomUUID(),
                    ...definition(agent),
                    broadcast,
                  },
                }),
              ),
            )
          }
        />
      </div>
      <div className="capability">
        <div>
          <p className="capability__name">Knowledge base</p>
          <p className="faint small">
            Lets {agent.name} search, read and write the wiki every person and
            agent shares. Off, it keeps nothing from there and cannot reach it.
          </p>
        </div>
        <Switch
          checked={agent.knowledge_base}
          label={`${agent.name} may use the knowledge base`}
          disabled={busy || agent.desired_state === 'retired'}
          onChange={(knowledge_base) =>
            void save(() =>
              ok(
                api.POST('/api/agents/{id}/definition', {
                  params: { path: { id: agent.id } },
                  body: {
                    request_id: crypto.randomUUID(),
                    ...definition(agent),
                    knowledge_base,
                  },
                }),
              ),
            )
          }
        />
      </div>
      <Alert>{error}</Alert>
    </section>
  )
}

/** The desired state an action would repeat, so the current one is disabled. */
const held: Partial<Record<LifecycleAction, string>> = {
  start: 'running',
  stop: 'stopped',
}
const ACTIONS: { action: LifecycleAction; label: string; danger?: boolean }[] =
  [
    { action: 'start', label: 'Start' },
    { action: 'stop', label: 'Stop' },
    { action: 'restart', label: 'Restart' },
    { action: 'retire', label: 'Retire', danger: true },
  ]

export function Lifecycle({
  agent,
  onChanged,
}: {
  agent: AgentSummary
  onChanged: () => Promise<unknown>
}) {
  const { save, error, notice, busy } = useSave(onChanged)
  const retired = agent.desired_state === 'retired'
  return (
    <section className="panel__section">
      <h2>Lifecycle</h2>
      <p className="panel__lead">
        Desired state: <strong>{agent.desired_state}</strong>. An agent's
        computer stays on: idle, it sleeps and dreams without stopping. Stop
        turns the computer off until you start it or a message arrives; the
        Status tab shows what actually runs. Stopping or retiring keeps the
        workspace and memory.
      </p>
      <div className="actions">
        {ACTIONS.map(({ action, label, danger }) => (
          <button
            key={action}
            type="button"
            className={danger ? 'btn btn--danger btn--sm' : 'btn btn--sm'}
            disabled={busy || retired || agent.desired_state === held[action]}
            onClick={() => {
              if (
                danger &&
                !window.confirm(
                  `Retire ${agent.name}? It cannot be started again.`,
                )
              )
                return
              void save(
                () => admin.lifecycle(agent.id, action, agent.revision),
                `${label} requested.`,
              )
            }}
          >
            {label}
          </button>
        ))}
      </div>
      <Alert>{error}</Alert>
      <Status>{notice}</Status>
    </section>
  )
}

function Tools({ agentId }: { agentId: string }) {
  const installation = useInstallation()
  const flags = installation.data
  return (
    <section className="panel__section">
      <h2>Tools</h2>
      <p className="panel__lead">
        Tool access is set for the whole installation in the controller
        configuration.
      </p>
      <dl className="kv">
        <dt>Terminal and files</dt>
        <dd>{flags?.terminal_enabled ? 'On' : 'Off'}</dd>
        <dt>Computer use and browser</dt>
        <dd>{flags?.computer_use_enabled ? 'On' : 'Off'}</dd>
        <dt>MCP servers</dt>
        <dd>
          <Link to="/agents/$agentId/connections" params={{ agentId }}>
            Manage in Connections
          </Link>
        </dd>
      </dl>
    </section>
  )
}
