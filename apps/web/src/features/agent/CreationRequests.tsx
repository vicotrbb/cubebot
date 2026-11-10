import { Bot, Hash, Network } from 'lucide-react'
import { useCallback, useEffect, useState } from 'react'
import {
  api,
  ApiError,
  failure,
  ok,
  type CreationRequest,
} from '../../api/client'
import type { AgentSummary } from '../../api/types'
import { Alert } from '../../components/primitives'

const ICONS = { agent: Bot, room: Hash, team: Network }

type Member = { agent?: string; title?: string; reports_to?: string | null }
/** The agent's tool input: untrusted, so every field is read defensively. */
type Input = {
  name?: unknown
  instructions?: unknown
  agents?: unknown
  description?: unknown
  parent?: unknown
  members?: unknown
}

const text = (value: unknown) => (typeof value === 'string' ? value : '')

/** An agent's pending requests to create agents, rooms and teams, each approved or declined here. */
export function CreationRequests({ agent }: { agent: AgentSummary }) {
  const [requests, setRequests] = useState<CreationRequest[]>([])
  const [failed, setFailed] = useState<{ id: string; text: string }[]>([])
  const refresh = useCallback(async () => {
    const { data, response } = await api.GET('/api/agents/{id}/creations', {
      params: { path: { id: agent.id } },
    })
    if (response.ok) setRequests(data ?? [])
  }, [agent.id])
  const pending = agent.pending_creations
  useEffect(() => {
    if (!pending) return
    const poll = () => void refresh().catch(() => undefined)
    poll()
    const timer = setInterval(poll, 3000)
    return () => clearInterval(timer)
  }, [refresh, pending])
  // A settled count hides what the last poll saw until the next request arrives.
  const shown = pending ? requests : []
  if (shown.length === 0 && failed.length === 0) return null
  return (
    <section
      className="creation-requests"
      aria-label={`Requests from ${agent.name}`}
    >
      {failed.map((notice) => (
        <div key={notice.id} className="creation-failed" role="alert">
          <p>{notice.text}</p>
          <button
            type="button"
            className="btn btn--ghost btn--sm"
            onClick={() =>
              setFailed((all) => all.filter((item) => item.id !== notice.id))
            }
          >
            Dismiss
          </button>
        </div>
      ))}
      {shown.map((request) => (
        <Request
          key={request.id}
          agent={agent}
          request={request}
          onSettled={(reason) => {
            setRequests((all) => all.filter((item) => item.id !== request.id))
            if (reason)
              setFailed((all) => [...all, { id: request.id, text: reason }])
          }}
        />
      ))}
    </section>
  )
}

function Request({
  agent,
  request,
  onSettled,
}: {
  agent: AgentSummary
  request: CreationRequest
  onSettled: (failed?: string) => void
}) {
  const input = request.input as Input
  const kind = request.kind as keyof typeof ICONS
  const Icon = ICONS[kind] ?? Bot
  const name = text(input.name)
  const [declining, setDeclining] = useState(false)
  const [reason, setReason] = useState('')
  const [error, setError] = useState('')
  const [busy, setBusy] = useState(false)
  const path = { path: { id: agent.id, request: request.id } }
  async function approve() {
    setBusy(true)
    setError('')
    const { error: refused, response } = await api.POST(
      '/api/agents/{id}/creations/{request}/approve',
      { params: path },
    )
    setBusy(false)
    if (response.ok) return onSettled()
    if (response.status === 409)
      return onSettled(
        `The ${kind} ${name} could not be created: ${(refused as unknown as { error?: string } | undefined)?.error ?? 'the platform refused it'}. ${agent.name} was told why.`,
      )
    setError(
      response.status === 403
        ? `Only someone allowed to create a ${kind} can approve this; for rooms and teams, an administrator.`
        : new ApiError(response.status).message,
    )
  }
  async function decline() {
    setBusy(true)
    setError('')
    try {
      await ok(
        api.POST('/api/agents/{id}/creations/{request}/decline', {
          params: path,
          body: { reason },
        }),
      )
      onSettled()
    } catch (error) {
      setError(failure(error))
      setBusy(false)
    }
  }
  return (
    <article className="creation-request">
      <span className="creation-request__icon" aria-hidden="true">
        <Icon />
      </span>
      <div className="creation-request__body">
        <p className="creation-request__title">
          {agent.name} asks to create the {kind} <strong>{name}</strong>
        </p>
        <Details kind={kind} input={input} />
      </div>
      {declining ? (
        <form
          className="creation-request__decline"
          onSubmit={(event) => {
            event.preventDefault()
            void decline()
          }}
        >
          <input
            className="input"
            aria-label="Reason for declining, sent to the agent"
            value={reason}
            maxLength={2048}
            // Focus follows the person's own click on Decline.
            autoFocus
            placeholder={`Tell ${agent.name} why (optional)`}
            onChange={(event) => setReason(event.target.value)}
          />
          <button
            type="submit"
            className="btn btn--sm btn--danger"
            disabled={busy}
          >
            {busy ? 'Declining…' : 'Decline'}
          </button>
          <button
            type="button"
            className="btn btn--ghost btn--sm"
            onClick={() => setDeclining(false)}
          >
            Cancel
          </button>
        </form>
      ) : (
        <div className="creation-request__actions">
          <button
            type="button"
            className="btn btn--ghost btn--sm"
            disabled={busy}
            onClick={() => setDeclining(true)}
          >
            Decline
          </button>
          <button
            type="button"
            className="btn btn--primary btn--sm"
            disabled={busy}
            onClick={() => void approve()}
          >
            {busy ? 'Creating…' : `Create ${kind}`}
          </button>
        </div>
      )}
      {error && (
        <div className="creation-request__error">
          <Alert>{error}</Alert>
        </div>
      )}
    </article>
  )
}

function Details({ kind, input }: { kind: string; input: Input }) {
  if (kind === 'room') {
    const agents = Array.isArray(input.agents) ? input.agents.map(text) : []
    return (
      <p className="creation-request__summary">
        {agents.length ? `With ${agents.join(', ')}` : 'With no agents yet'}
      </p>
    )
  }
  if (kind === 'team') return <TeamDetails input={input} />
  const instructions = text(input.instructions)
  return (
    <details className="creation-request__more">
      <summary>Read its instructions</summary>
      <div className="creation-request__text">
        {instructions || 'No instructions.'}
      </div>
    </details>
  )
}

function TeamDetails({ input }: { input: Input }) {
  const members = (Array.isArray(input.members) ? input.members : []).filter(
    (member): member is Member => typeof member === 'object' && !!member,
  )
  const parent = text(input.parent)
  return (
    <>
      <p className="creation-request__summary">
        {text(input.description)}
        {parent && ` Under ${parent}.`}
      </p>
      <ul className="creation-request__roster">
        {members.map((member, index) => (
          <li key={index}>
            <span>{text(member.agent)}</span>
            <span className="faint">
              {text(member.title)}
              {member.reports_to
                ? ` · reports to ${text(member.reports_to)}`
                : ' · manager'}
            </span>
          </li>
        ))}
      </ul>
    </>
  )
}
