import { useQuery, useQueryClient } from '@tanstack/react-query'
import { ChevronDown, LogIn, RotateCw, Trash2, Zap } from 'lucide-react'
import { useState } from 'react'
import { api, failure } from '../../api/client'
import { useAgents } from '../../app/queries'
import {
  Alert,
  confirmAction,
  Status,
  Switch,
  when,
} from '../../components/primitives'
import { answer, CONNECTIONS_KEY } from './api'
import {
  type Connection,
  type Level,
  type Manifest,
  grouped,
  LEVELS,
  permitted,
  RISK_LABEL,
} from './model'

type Change = {
  level?: Level
  disabled_tools?: string[]
  access?: Connection['access']
}

const STATUS: Record<string, string> = {
  connected: 'Connected',
  sign_in_again: 'Needs sign-in again',
  error: 'Error',
}

export function Account({
  manifest,
  connection,
  expanded,
  onToggle,
  onSignInAgain,
}: {
  manifest: Manifest
  connection: Connection
  expanded: boolean
  onToggle: () => void
  onSignInAgain: () => void
}) {
  const client = useQueryClient()
  const [busy, setBusy] = useState('')
  const [error, setError] = useState('')
  const [notice, setNotice] = useState('')
  const path = { params: { path: { id: connection.id } } }
  const act = async (what: string, run: () => Promise<unknown>, done = '') => {
    setBusy(what)
    setError('')
    setNotice('')
    try {
      await run()
      setNotice(done)
    } catch (caught) {
      setError(failure(caught))
    } finally {
      setBusy('')
      await client.invalidateQueries({ queryKey: CONNECTIONS_KEY })
    }
  }
  const change = (body: Change) =>
    act('save', () =>
      answer(api.PUT('/api/connections/{id}', { ...path, body })),
    )
  const test = () =>
    act(
      'test',
      async () => {
        const checked = await answer(
          api.POST('/api/connections/{id}/check', path),
        )
        if (checked && checked.status !== 'connected')
          throw new Error(checked.problem ?? 'The check failed.')
      },
      'The account answered; the connection works.',
    )
  const remove = async () => {
    const yes = await confirmAction({
      title: `Remove ${connection.slug}?`,
      body: 'Agents lose its tools from their next call, and routines waiting on its events stop running. An OAuth sign-in is revoked at the vendor.',
      action: 'Remove',
    })
    if (yes)
      await act('remove', () =>
        answer(api.DELETE('/api/connections/{id}', path)),
      )
  }
  const vendor =
    connection.auth === 'oauth2' || connection.auth === 'github_app'
  return (
    <li className="account" data-status={connection.status}>
      <button
        type="button"
        className="account__summary"
        aria-expanded={expanded}
        onClick={onToggle}
      >
        <span className="account__dot" aria-hidden="true" />
        <span className="account__names">
          <span className="account__slug">{connection.slug}</span>
          <span className="account__account">
            {connection.account ?? 'Account not named yet'} ·{' '}
            {STATUS[connection.status] ?? connection.status}
          </span>
        </span>
        <span className="chip">
          {LEVELS.find((l) => l.id === connection.level)?.label}
        </span>
        <ChevronDown className="account__chevron" aria-hidden="true" />
      </button>
      {expanded && (
        <div className="account__body">
          {connection.problem && (
            <p className="notice notice--danger">{connection.problem}</p>
          )}
          <div className="actions">
            <button
              type="button"
              className="btn btn--sm"
              disabled={Boolean(busy)}
              onClick={() => void test()}
            >
              <RotateCw className={busy === 'test' ? 'spin' : undefined} />
              {busy === 'test' ? 'Testing…' : 'Test connection'}
            </button>
            {(vendor || replaceable(connection)) && (
              <button
                type="button"
                className="btn btn--sm"
                onClick={onSignInAgain}
              >
                <LogIn /> {vendor ? 'Sign in again' : 'Replace credential'}
              </button>
            )}
            <button
              type="button"
              className="btn btn--ghost btn--sm account__remove"
              disabled={Boolean(busy)}
              onClick={() => void remove()}
            >
              <Trash2 /> Remove
            </button>
          </div>
          <Status>{notice}</Status>
          <Alert>{error}</Alert>
          <p className="small faint">
            {connection.checked_at
              ? `Last checked ${when(connection.checked_at)}.`
              : 'Not checked yet.'}{' '}
            {connection.expires_at
              ? `Its access token renews before ${when(connection.expires_at)}.`
              : ''}
          </p>
          <Permission
            connection={connection}
            busy={Boolean(busy)}
            onChange={change}
          />
          <Access
            connection={connection}
            busy={Boolean(busy)}
            onChange={change}
          />
          <Tools
            manifest={manifest}
            connection={connection}
            busy={Boolean(busy)}
            onChange={change}
          />
          {manifest.events.length > 0 && (
            <Events manifest={manifest} connection={connection} />
          )}
        </div>
      )}
    </li>
  )
}

function Permission({
  connection,
  busy,
  onChange,
}: {
  connection: Connection
  busy: boolean
  onChange: (change: Change) => void
}) {
  const current = LEVELS.find((level) => level.id === connection.level)
  return (
    <section className="account__part">
      <h4>Permission level</h4>
      <div
        className="tabs level"
        role="radiogroup"
        aria-label="Permission level"
      >
        {LEVELS.map((level) => (
          <button
            key={level.id}
            type="button"
            role="radio"
            aria-checked={level.id === connection.level}
            disabled={busy}
            onClick={() => onChange({ level: level.id })}
          >
            {level.label}
          </button>
        ))}
      </div>
      <p className="small faint">
        {current?.meaning} Agents act without asking, so this is the limit.
      </p>
    </section>
  )
}

function Access({
  connection,
  busy,
  onChange,
}: {
  connection: Connection
  busy: boolean
  onChange: (change: Change) => void
}) {
  const agents = useAgents()
  const everyone = connection.access.mode === 'all_agents'
  const granted =
    connection.access.mode === 'explicit_grants' ? connection.access.agents : []
  const grant = (id: string, on: boolean) =>
    onChange({
      access: {
        mode: 'explicit_grants',
        agents: on ? [...granted, id] : granted.filter((agent) => agent !== id),
      },
    })
  const live = (agents.data ?? []).filter(
    (agent) => agent.desired_state !== 'retired',
  )
  return (
    <section className="account__part">
      <div className="account__row">
        <h4>Every agent may use it</h4>
        <Switch
          checked={everyone}
          disabled={busy}
          label={`Every agent may use ${connection.slug}`}
          onChange={(on) =>
            onChange({
              access: on
                ? { mode: 'all_agents' }
                : { mode: 'explicit_grants', agents: [] },
            })
          }
        />
      </div>
      {!everyone && (
        <ul className="account__agents" aria-label="Agents that may use it">
          {live.length === 0 && <li className="small faint">No agents yet.</li>}
          {live.map((agent) => (
            <li key={agent.id}>
              <label className="check">
                <input
                  type="checkbox"
                  checked={granted.includes(agent.id)}
                  disabled={busy}
                  onChange={(event) => grant(agent.id, event.target.checked)}
                />
                <span>{agent.name}</span>
              </label>
            </li>
          ))}
        </ul>
      )}
    </section>
  )
}

function Tools({
  manifest,
  connection,
  busy,
  onChange,
}: {
  manifest: Manifest
  connection: Connection
  busy: boolean
  onChange: (change: Change) => void
}) {
  const products = Array.isArray(connection.config.products)
    ? (connection.config.products as string[])
    : null
  const disabled = connection.disabled_tools
  const toggle = (name: string, on: boolean) =>
    onChange({
      disabled_tools: on
        ? disabled.filter((tool) => tool !== name)
        : [...disabled, name],
    })
  return (
    <section className="account__part">
      <h4>Tools</h4>
      <p className="small faint">
        Agents call them as <code>{connection.slug}__name</code>. Ones above the
        level stay off.
      </p>
      {grouped(manifest, products).map((group) => (
        <div key={group.risk} className="tools">
          <h5>{RISK_LABEL[group.risk]}</h5>
          <ul>
            {group.tools.map((tool) => {
              const allowed = permitted(connection.level, tool.risk)
              return (
                <li key={tool.name} className="tool" data-allowed={allowed}>
                  <div className="tool__text">
                    <code>{tool.name}</code>
                    <span>{plain(tool.description)}</span>
                  </div>
                  <Switch
                    checked={allowed && !disabled.includes(tool.name)}
                    disabled={busy || !allowed}
                    label={`${tool.name} ${allowed ? 'is offered' : 'is above the permission level'}`}
                    onChange={(on) => toggle(tool.name, on)}
                  />
                </li>
              )
            })}
          </ul>
        </div>
      ))}
    </section>
  )
}

/** Descriptions are written for models, with code in backticks. */
function plain(text: string) {
  return text.replaceAll('`', '')
}

function Events({
  manifest,
  connection,
}: {
  manifest: Manifest
  connection: Connection
}) {
  const activity = useQuery({
    queryKey: ['connection-activity', connection.id],
    queryFn: () =>
      answer(
        api.GET('/api/connections/{id}/activity', {
          params: { path: { id: connection.id } },
        }),
      ),
    refetchInterval: 60_000,
  })
  const routines = activity.data?.routines ?? []
  const recent = activity.data?.events ?? []
  return (
    <section className="account__part">
      <h4>Events</h4>
      <ul className="events">
        {manifest.events.map((event) => (
          <li key={event.kind}>
            <Zap aria-hidden="true" />
            <span>
              <code>{event.kind}</code> {event.description}
            </span>
          </li>
        ))}
      </ul>
      {routines.length === 0 ? (
        <p className="small faint">
          No routine waits on these yet. On an agent’s Routines tab, choose
          “When a connected account reports an event”.
        </p>
      ) : (
        <ul className="events__routines" aria-label="Routines it starts">
          {routines.map((routine) => (
            <li key={routine.id} className="small">
              <strong>{routine.name}</strong> on {routine.agent_name} runs on{' '}
              <code>{routine.event}</code>
              {!routine.enabled && ' (paused)'}
            </li>
          ))}
        </ul>
      )}
      {recent.length > 0 && (
        <ul className="events__recent" aria-label="Recent events">
          {recent.map((event) => (
            <li key={`${event.kind}${event.received_at}${event.summary}`}>
              <span className="events__when">{when(event.received_at)}</span>
              <span className="events__summary">{event.summary}</span>
              {event.routines > 0 && (
                <span className="chip chip--sage">
                  {event.routines === 1 ? '1 run' : `${event.routines} runs`}
                </span>
              )}
            </li>
          ))}
        </ul>
      )}
    </section>
  )
}

/** A refused credential can be pasted again; a public connection has none. */
function replaceable(connection: Connection) {
  return connection.status === 'sign_in_again' && connection.auth !== 'public'
}
