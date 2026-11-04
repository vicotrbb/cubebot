import { createFileRoute } from '@tanstack/react-router'
import { useState } from 'react'
import { api, failure, messageProblem, ok, rejectedInput } from '../api/client'
import { can } from '../api/types'
import { Alert, Empty } from '../components/primitives'
import { useAgentScreen } from '../features/agent/context'
import { Steering, Stop } from '../features/chat/Conversation'
import { Markdown } from '../features/chat/Markdown'
import { isOpen, isUnsettled } from '../features/chat/useConversation'

export const Route = createFileRoute('/_app/agents/$agentId/thread/$turnId')({
  component: Thread,
})

const STATES: Record<string, string> = {
  queued: 'queued',
  running: 'working',
  waiting: 'waiting for a person',
  delegated: 'waiting on its own delegates',
  complete: 'complete',
  failed: 'failed',
  cancelled: 'cancelled',
}

/** One turn's delegated work and steering, like a Slack thread. */
function Thread() {
  const { turnId } = Route.useParams()
  const { agent, session, turns } = useAgentScreen()
  const turn = turns.find((item) => item.id === turnId)
  if (!turn)
    return (
      <Empty title="This message is not loaded.">
        Show earlier messages first.
      </Empty>
    )
  return (
    <>
      <div className="thread__root">
        <p className="small faint">Message #{turn.sequence}</p>
        <Markdown text={turn.message} />
      </div>
      <section className="panel__section">
        <h2>Delegated work</h2>
        {turn.children.length === 0 && (
          <p className="panel__lead">Nothing delegated from this message.</p>
        )}
        <ul aria-label="Delegated work">
          {turn.children.map((child) => (
            <li key={child.id} className="thread__child">
              <div className="title">
                <strong>{child.agent}</strong>
                <span className="chip">
                  {child.awaited ? 'delegated' : 'message'}
                </span>
                <span className="faint small">
                  {STATES[child.status] ?? child.status}
                </span>
              </div>
              {child.error && child.status !== 'cancelled' && (
                <p className="small" style={{ color: 'var(--danger)' }}>
                  {child.error.replaceAll('_', ' ')}
                </p>
              )}
              {child.response && <Markdown text={child.response} />}
            </li>
          ))}
        </ul>
      </section>
      <section className="panel__section">
        <h2>Steering</h2>
        <Steering turn={turn} />
        {isOpen(turn) && can(agent, 'conversation_send', session) ? (
          <Steer key={turn.id} conversation={agent.id} turnId={turn.id} />
        ) : (
          <p className="panel__lead">
            {isOpen(turn)
              ? 'You cannot steer this work.'
              : 'Steering is for work that is still running.'}
          </p>
        )}
        {isUnsettled(turn) && <Stop conversation={agent.id} turn={turn} />}
      </section>
    </>
  )
}

/** A correction for this turn only; ordinary messages still queue. */
function Steer({
  conversation,
  turnId,
}: {
  conversation: string
  turnId: string
}) {
  const [text, setText] = useState('')
  const [requestId, setRequestId] = useState('')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  return (
    <form
      className="form"
      onSubmit={async (event) => {
        event.preventDefault()
        const problem = messageProblem(text)
        if (problem) return setError(problem)
        setBusy(true)
        setError('')
        const id = requestId || crypto.randomUUID()
        setRequestId(id)
        try {
          await ok(
            api.POST(
              '/api/conversations/{conversation}/messages/{turn}/steer',
              {
                params: { path: { conversation, turn: turnId } },
                body: { request_id: id, message: text },
              },
            ),
          )
          setText('')
          setRequestId('')
        } catch (error) {
          if (rejectedInput(error)) setRequestId('')
          setError(failure(error))
        } finally {
          setBusy(false)
        }
      }}
    >
      <label className="field">
        <span>Steer this work</span>
        <textarea
          value={text}
          rows={3}
          maxLength={16384}
          placeholder="A correction, applied at the next safe point"
          onChange={(event) => {
            setText(event.target.value)
            setRequestId('')
          }}
        />
      </label>
      <Alert>{error}</Alert>
      <div className="actions">
        <button
          className="btn btn--primary btn--sm"
          disabled={busy || !text.trim()}
        >
          {busy ? 'Steering…' : 'Steer'}
        </button>
      </div>
    </form>
  )
}
