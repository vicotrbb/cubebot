import { Link } from '@tanstack/react-router'
import { ChevronRight, ListChecks, Megaphone, Repeat } from 'lucide-react'
import { useEffect, useRef, useState } from 'react'
import { api, failure, ok, type Turn } from '../../api/client'
import type { AgentSummary } from '../../api/types'
import { Cube, moodOf } from '../../components/Cube'
import { PersonAvatar } from '../../components/PersonAvatar'
import { Alert, Skeleton } from '../../components/primitives'
import { useFollowEnd } from './useFollowEnd'
import { CallMessage } from '../call/CallMessage'
import { Attachments } from './Attachments'
import { Composer } from './Composer'
import { Markdown } from './Markdown'
import { isOpen, isUnsettled, useConversation } from './useConversation'
import { WorkCard, WorkLog } from './WorkCard'
import { ApprovalBar, AskRow, isAsk, useApprovals } from './Approvals'
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
  useFollowEnd(scroller, pinned)
  const visible = turns.slice(Math.max(0, turns.length - shown))
  const last = turns.at(-1)
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
        {visible.map((turn) =>
          turn.call ? (
            <CallMessage key={turn.id} call={turn.call}>
              {turn.call.turns.map((said) => (
                <Exchange
                  key={said.id}
                  agent={agent}
                  turn={said}
                  streaming={undefined}
                  you={you}
                  canSend={canSend}
                  onOpenThread={() => onOpenThread(said)}
                />
              ))}
            </CallMessage>
          ) : (
            <Exchange
              key={turn.id}
              agent={agent}
              turn={turn}
              streaming={streaming[turn.id]}
              you={you}
              canSend={canSend}
              onOpenThread={() => onOpenThread(turn)}
            />
          ),
        )}
      </div>
      <Alert>{readError}</Alert>
      {canSend ? (
        <Composer
          draftKey={`cubebot:${accountId}:${id}`}
          label="Send message"
          placeholder={`Message ${agent.name}`}
          note={
            last && !last.call && isOpen(last)
              ? queuedLabel(agent.activity).replace(
                  'Queued',
                  'Your message queues',
                )
              : 'Enter sends · Shift+Enter for a new line'
          }
          attachable
          send={async (request_id, message, attachments) => {
            const turn = await ok(
              api.POST('/api/conversations/{conversation}/messages', {
                params: { path: { conversation: id } },
                body: { request_id, message, attachments },
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
  if (turn.status === 'waiting') return waitingLabel(turn)
  return ''
}

const WAITING: Record<string, string> = {
  budget_exhausted:
    'Paused: this agent spent its budget. It continues the next day or month the budget allows, or when someone raises it under Usage.',
  provider_usage_limit:
    'Waiting: the signed-in plan reached its usage limit. Retry once the plan’s usage resets, or switch agents to another provider.',
  provider_invalid_response:
    'Waiting: the provider’s reply could not be read. Retry the response; if it keeps happening, choose another model.',
}

function waitingLabel(turn: Turn) {
  if (turn.terminal.some((t) => t.status === 'uncertain'))
    return 'Terminal effects are uncertain. Stop this turn, then ask the agent to inspect the workspace in a new message.'
  const error = turn.error ?? 'execution interrupted'
  return (
    WAITING[error] ??
    `Waiting: ${error.replaceAll('_', ' ')}. Retry when ready.`
  )
}

function queuedLabel(activity: string) {
  if (activity === 'working') return 'Queued until the agent is free.'
  if (activity === 'stopped')
    return 'Queued; the agent takes it once its computer starts.'
  return 'Queued; waiting for the agent’s runtime.'
}

function origin(turn: Turn) {
  const from = turn.origin
  if (!from) return null
  if (from.kind === 'approvals') return `${from.name} settled the requests`
  if (from.kind === 'broadcast') return `broadcast from ${from.name}`
  if (from.kind === 'routine') return `via routine ${from.name}`
  if (from.kind === 'delegation') return `delegated by ${from.name}`
  return `message from ${from.name}`
}

function Exchange({
  agent,
  turn,
  streaming,
  you,
  canSend,
  onOpenThread,
}: {
  agent: AgentSummary
  turn: Turn
  streaming: string | undefined
  you: string
  canSend: boolean
  onOpenThread: () => void
}) {
  const approvals = useApprovals(agent)
  const asking = turn.terminal.some(
    (tool) => isAsk(tool) && approvals.statusOf(tool).status === 'pending',
  )
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
          {canSend && (
            <ApprovalBar
              agent={agent}
              tools={turn.terminal}
              approvals={approvals}
            />
          )}
          {turn.terminal.length > 0 && (
            <WorkLog
              tools={turn.terminal}
              running={turn.status === 'running'}
              open={turn.status === 'running' || asking}
            >
              {turn.terminal.map((tool) => (
                <div key={tool.id}>
                  {tool.reobserved && (
                    <p className="desktop-marker small faint">
                      Control returned · agent re-observed the screen
                    </p>
                  )}
                  {isAsk(tool) ? (
                    <AskRow
                      agent={agent}
                      tool={tool}
                      approvals={approvals}
                      canSettle={canSend}
                    />
                  ) : (
                    <WorkCard tool={tool} />
                  )}
                </div>
              ))}
            </WorkLog>
          )}
          {turn.response ? (
            <Markdown text={turn.response} />
          ) : (
            streaming && <Markdown text={streaming} />
          )}
          <Attachments turn={turn.id} items={turn.attachments} />
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

function shownRequest(turn: Turn) {
  if (turn.origin?.kind === 'broadcast')
    return turn.message.replace(/^Broadcast [^\n]*:\n\n/, '')
  if (turn.origin?.kind === 'routine')
    return turn.message.replace(/^Routine [^\n]*:\n\n/, '')
  if (turn.origin?.kind === 'approvals')
    return turn.message.replace(/^[^\n]*\n/, '')
  return turn.message
}

function PersonMessage({ turn, you }: { turn: Turn; you: string }) {
  const via = origin(turn)
  const from = turn.origin
  return (
    <div className="msg msg--person">
      {from ? (
        <span className="avatar-initial msg__avatar" aria-hidden="true">
          {from.kind === 'routine' && <Repeat size={16} />}
          {from.kind === 'broadcast' && <Megaphone size={16} />}
          {from.kind === 'approvals' && <ListChecks size={16} />}
          {(from.kind === 'delegation' || from.kind === 'message') &&
            from.name.slice(0, 1).toUpperCase()}
        </span>
      ) : (
        <PersonAvatar login={you} className="msg__avatar" />
      )}
      <div className="msg__body">
        {from?.kind === 'approvals' ? (
          <Settled who={capital(via ?? '')} text={shownRequest(turn)} />
        ) : (
          <>
            <div className="msg__head">
              <span className="msg__author">{via ? capital(via) : you}</span>
            </div>
            <Markdown text={shownRequest(turn)} />
            <Attachments turn={turn.id} items={turn.message_attachments} />
          </>
        )}
      </div>
    </div>
  )
}

function Settled({ who, text }: { who: string; text: string }) {
  const lines = text.split('\n').filter(Boolean)
  const count = (ending: RegExp) =>
    lines.filter((line) => ending.test(line)).length
  const parts = [
    [count(/: (created|made)\.$/), 'approved'],
    [count(/: declined: /), 'declined'],
    [count(/: could not be made: /), 'failed'],
  ] as const
  const outcome = parts
    .filter(([n]) => n > 0)
    .map(([n, word]) => `${n} ${word}`)
    .join(', ')
  return (
    <details className="settled">
      <summary className="settled__summary">
        <span className="msg__author">{who}</span>
        <span className="faint small">{outcome}</span>
        <ChevronRight className="settled__chevron" aria-hidden="true" />
      </summary>
      <Markdown text={text} />
    </details>
  )
}

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
      {turn.status === 'waiting' &&
        turn.error !== 'budget_exhausted' &&
        !uncertain && <Retry conversation={agent.id} turn={turn} />}
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

export function Retry({
  conversation,
  turn,
}: {
  conversation: string
  turn: Pick<Turn, 'id'>
}) {
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

export function Stop({
  conversation,
  turn,
  label = 'Stop',
}: {
  conversation: string
  turn: Pick<Turn, 'id' | 'status'>
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
