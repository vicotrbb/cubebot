import { Link } from '@tanstack/react-router'
import { Hash, Megaphone, Repeat } from 'lucide-react'
import { useEffect, useRef, useState } from 'react'
import { api, failure, ok, type Turn } from '../../api/client'
import type { AgentSummary } from '../../api/types'
import { Cube, moodOf } from '../../components/Cube'
import { PersonAvatar } from '../../components/PersonAvatar'
import { Alert, Skeleton } from '../../components/primitives'
import { Composer } from './Composer'
import { isOpen, isUnsettled, useConversation } from './useConversation'
import { WorkCard, WorkLog } from './WorkCard'
import './chat.css'

const PAGE = 60

export function Conversation({
  agent,
  accountId,
  you,
  canSend,
  turns,
  streaming,
  loaded,
  readError,
  add,
  onOpenThread,
}: {
  agent: AgentSummary
  accountId: string
  you: string
  canSend: boolean
  onOpenThread: (turn: Turn) => void
} & ReturnType<typeof useConversation>) {
  const id = agent.id
  const [shown, setShown] = useState(PAGE)
  const scroller = useRef<HTMLDivElement>(null)
  const pinned = useRef(true)
  const visible = turns.slice(Math.max(0, turns.length - shown))
  const last = turns.at(-1)
  // Follow the newest turn unless the reader scrolled up to read history.
  useEffect(() => {
    const element = scroller.current
    if (element && pinned.current) element.scrollTop = element.scrollHeight
  }, [turns.length, streaming, last?.status, loaded])
  return (
    <section className="conversation" aria-label="Conversation">
      <div
        ref={scroller}
        className="transcript"
        tabIndex={0}
        aria-label="Message history"
        onScroll={(event) => {
          const element = event.currentTarget
          pinned.current =
            element.scrollHeight - element.scrollTop - element.clientHeight < 80
        }}
      >
        {!loaded && (
          <div
            className="transcript__loading"
            role="status"
            aria-label="Loading history"
          >
            <Skeleton lines={4} width="60%" />
          </div>
        )}
        {loaded && turns.length === 0 && (
          <div className="empty transcript__empty">
            <Cube avatar={agent.avatar} mood={moodOf(agent)} size={72} follow />
            <h2>{agent.name} is ready.</h2>
            <p>
              Send a message. Cubebot saves it before the agent begins, and the
              whole history stays in this one conversation.
            </p>
          </div>
        )}
        {loaded && turns.length > shown && (
          <button
            type="button"
            className="btn btn--sm transcript__earlier"
            onClick={() => {
              pinned.current = false
              setShown((count) => count + PAGE)
            }}
          >
            Show earlier messages
          </button>
        )}
        {visible.map((turn) => (
          <Exchange
            key={turn.id}
            agent={agent}
            turn={turn}
            streaming={streaming[turn.id]}
            you={you}
            onOpenThread={() => onOpenThread(turn)}
          />
        ))}
      </div>
      <Alert>{readError}</Alert>
      {canSend ? (
        <Composer
          draftKey={`cubebot:${accountId}:${id}`}
          label="Send message"
          placeholder={`Message ${agent.name}`}
          note={
            last && isOpen(last)
              ? queuedLabel(agent.activity).replace(
                  'Queued',
                  'Your message queues',
                )
              : 'Enter sends · Shift+Enter for a new line'
          }
          send={async (request_id, message) => {
            const turn = await ok(
              api.POST('/api/conversations/{conversation}/messages', {
                params: { path: { conversation: id } },
                body: { request_id, message },
              }),
            )
            add(turn)
          }}
        />
      ) : (
        <p className="composer composer--readonly faint small">
          You can read this conversation but not send to it.
        </p>
      )}
    </section>
  )
}

function stateLabel(turn: Turn, activity: string) {
  if (turn.status === 'queued') return queuedLabel(activity)
  if (turn.status === 'running') return 'Working…'
  if (turn.status === 'delegated')
    return 'Waiting for delegated work; resumes as each task settles.'
  if (turn.status === 'failed')
    return `Failed: ${(turn.error ?? 'unknown').replaceAll('_', ' ')}.`
  if (turn.status === 'cancelled')
    return 'Cancelled, with any work delegated under it. Workspace changes may already have occurred.'
  if (
    turn.status === 'waiting' &&
    turn.terminal.some((t) => t.status === 'uncertain')
  )
    return 'Terminal effects are uncertain. Stop this turn, then ask the agent to inspect the workspace in a new message.'
  if (turn.status === 'waiting')
    return `Waiting: ${(turn.error ?? 'execution interrupted').replaceAll('_', ' ')}. Retry when ready.`
  return ''
}

/** Why a queued turn waits: the same state the header shows, not a guess. */
function queuedLabel(activity: string) {
  if (activity === 'working') return 'Queued until the agent is free.'
  if (activity === 'stopped')
    return 'Queued; the agent takes it once its computer starts.'
  return 'Queued; waiting for the agent’s runtime.'
}

function origin(turn: Turn) {
  const from = turn.origin
  if (!from) return null
  if (from.kind === 'broadcast') return `broadcast from ${from.name}`
  if (from.kind === 'room') return `via room ${from.name}`
  if (from.kind === 'routine') return `via routine ${from.name}`
  if (from.kind === 'delegation') return `delegated by ${from.name}`
  return `message from ${from.name}`
}

function Exchange({
  agent,
  turn,
  streaming,
  you,
  onOpenThread,
}: {
  agent: AgentSummary
  turn: Turn
  streaming: string | undefined
  you: string
  onOpenThread: () => void
}) {
  return (
    <article className="exchange" data-status={turn.status}>
      <PersonMessage turn={turn} you={you} />
      <div className="msg msg--agent agent-message">
        <Cube
          agentId={agent.id}
          avatar={agent.avatar}
          mood={turn.status === 'running' ? 'working' : 'idle'}
          size={32}
          className="msg__avatar"
        />
        <div className="msg__body">
          <div className="msg__head">
            <span className="msg__author">{agent.name}</span>
            <span className="msg__state faint small">
              {stateLabel(turn, agent.activity)}
            </span>
          </div>
          {turn.terminal.length > 0 && (
            <WorkLog tools={turn.terminal} running={turn.status === 'running'}>
              {turn.terminal.map((tool) => (
                <div key={tool.id}>
                  {tool.reobserved && (
                    <p className="desktop-marker small faint">
                      Control returned · agent re-observed the screen
                    </p>
                  )}
                  <WorkCard tool={tool} />
                </div>
              ))}
            </WorkLog>
          )}
          {turn.response ? (
            <p className="msg__text">{turn.response}</p>
          ) : (
            streaming && <p className="msg__text">{streaming}</p>
          )}
          {turn.children.length > 0 && (
            <ThreadLine turn={turn} onOpen={onOpenThread} />
          )}
          <Steering turn={turn} />
          <TurnActions agent={agent} turn={turn} onOpenThread={onOpenThread} />
        </div>
      </div>
    </article>
  )
}

/** The request as a person's message, or as the room, routine or agent it came from. */
function PersonMessage({ turn, you }: { turn: Turn; you: string }) {
  const via = origin(turn)
  const from = turn.origin
  return (
    <div className="msg msg--person">
      {from ? (
        <span className="avatar-initial msg__avatar" aria-hidden="true">
          {from.kind === 'room' && <Hash size={16} />}
          {from.kind === 'routine' && <Repeat size={16} />}
          {from.kind === 'broadcast' && <Megaphone size={16} />}
          {(from.kind === 'delegation' || from.kind === 'message') &&
            from.name.slice(0, 1).toUpperCase()}
        </span>
      ) : (
        <PersonAvatar login={you} className="msg__avatar" />
      )}
      <div className="msg__body">
        <div className="msg__head">
          <span className="msg__author">{via ? capital(via) : you}</span>
          {from && (from.kind === 'routine' || from.kind === 'room') && (
            <span className="chip">
              {CHIPS[from.kind]} · {from.name}
            </span>
          )}
        </div>
        <p className="msg__text">
          {from?.kind === 'broadcast'
            ? turn.message.replace(/^Broadcast [^\n]*:\n\n/, '')
            : turn.message}
        </p>
      </div>
    </div>
  )
}

/** "3 tasks · 2 done": the delegated work under a turn, opened like a thread. */
function ThreadLine({ turn, onOpen }: { turn: Turn; onOpen: () => void }) {
  const done = turn.children.filter((child) =>
    ['complete', 'failed', 'cancelled'].includes(child.status),
  ).length
  return (
    <button
      type="button"
      className="thread-line"
      onClick={onOpen}
      aria-label="Delegated work"
    >
      <span className="thread-line__faces">
        {turn.children.slice(0, 3).map((child) => (
          <span key={child.id} className="avatar-initial" aria-hidden="true">
            {child.agent.slice(0, 1)}
          </span>
        ))}
      </span>
      {turn.children.length} {turn.children.length === 1 ? 'task' : 'tasks'} ·{' '}
      {done} done
      {turn.status === 'delegated' && ' · waiting'}
    </button>
  )
}

function TurnActions({
  agent,
  turn,
  onOpenThread,
}: {
  agent: AgentSummary
  turn: Turn
  onOpenThread: () => void
}) {
  const uncertain = turn.terminal.some((tool) => tool.status === 'uncertain')
  return (
    <div className="msg__actions">
      {turn.status === 'waiting' && !uncertain && (
        <Retry conversation={agent.id} turn={turn} />
      )}
      {isOpen(turn) && (
        <button type="button" className="link-btn small" onClick={onOpenThread}>
          Steer this work
        </button>
      )}
      {isUnsettled(turn) && (
        <Stop
          conversation={agent.id}
          turn={turn}
          label={turn.status === 'queued' ? 'Cancel' : 'Stop'}
        />
      )}
      <Link
        to="/agents/$agentId/thread/$turnId"
        params={{ agentId: agent.id, turnId: turn.id }}
        className="small msg__time"
      >
        Thread #{turn.sequence}
      </Link>
    </div>
  )
}

const CHIPS: Record<string, string> = {
  routine: 'Routine',
  room: 'Room',
}

function capital(text: string) {
  return text.slice(0, 1).toUpperCase() + text.slice(1)
}

export function Steering({ turn }: { turn: Turn }) {
  const open = isOpen(turn)
  if (turn.steering.length === 0) return null
  return (
    <ul className="steering">
      {turn.steering.map((note) => (
        <li key={note.id} className="small">
          <strong>
            {note.propagated ? 'Steering from the delegating work' : 'Steering'}
          </strong>
          : “{note.message}” ·{' '}
          <span className="muted">
            {note.incorporated
              ? 'incorporated into the running work'
              : open
                ? 'accepted, waiting for the next safe point'
                : 'not incorporated; the work ended first'}
          </span>
        </li>
      ))}
    </ul>
  )
}

function Retry({ conversation, turn }: { conversation: string; turn: Turn }) {
  const [attempt, setAttempt] = useState<{ request_id: string } | null>(null)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  return (
    <>
      <button
        type="button"
        className="btn btn--sm"
        disabled={busy}
        onClick={async () => {
          setBusy(true)
          setError('')
          const input = attempt ?? { request_id: crypto.randomUUID() }
          setAttempt(input)
          try {
            await ok(
              api.POST(
                '/api/conversations/{conversation}/messages/{turn}/retry',
                {
                  params: { path: { conversation, turn: turn.id } },
                  body: input,
                },
              ),
            )
            setAttempt(null)
          } catch (error) {
            setError(failure(error))
          } finally {
            setBusy(false)
          }
        }}
      >
        {busy ? 'Retrying…' : 'Retry response'}
      </button>
      <Alert>{error}</Alert>
    </>
  )
}

/** The orchestrator delivers the cancel to the agent over its runtime channel. */
export function Stop({
  conversation,
  turn,
  label = 'Stop',
}: {
  conversation: string
  turn: Turn
  label?: string
}) {
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  return (
    <>
      <button
        type="button"
        className="btn btn--sm btn--danger"
        disabled={busy}
        onClick={async () => {
          setBusy(true)
          setError('')
          try {
            await ok(
              api.POST(
                '/api/conversations/{conversation}/messages/{turn}/cancel',
                {
                  params: { path: { conversation, turn: turn.id } },
                },
              ),
            )
          } catch (error) {
            setError(failure(error))
          } finally {
            setBusy(false)
          }
        }}
      >
        {busy ? 'Stopping…' : label}
      </button>
      <Alert>{error}</Alert>
    </>
  )
}
