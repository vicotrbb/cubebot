import { useEffect, useRef, useState } from 'react'
import { api, failure, history, ok, type RoomMessage } from '../../api/client'
import type { AgentSummary, RoomSummary, Session } from '../../api/types'
import { markRead, useAgents } from '../../app/queries'
import { followWhileVisible } from '../../app/stream'
import { Cube, moodOf } from '../../components/Cube'
import { PersonAvatar } from '../../components/PersonAvatar'
import { Alert, Empty, Skeleton } from '../../components/primitives'
import { can } from '../../api/types'
import { Attachments } from '../chat/Attachments'
import { Composer } from '../chat/Composer'
import { Retry, Stop } from '../chat/Conversation'
import { Markdown } from '../chat/Markdown'
import { WorkCard, WorkLog } from '../chat/WorkCard'
import '../chat/chat.css'
import { ORCHESTRATOR_AVATAR } from '../tasks/orchestrator'
import { rounds } from './rounds'

const FINISHED = ['complete', 'failed', 'cancelled']

/** Applies message snapshots by id, in room order; a finished delivery never goes back. */
function merge(known: RoomMessage[], incoming: RoomMessage[]) {
  const byId = new Map(known.map((message) => [message.id, message]))
  for (const message of incoming) {
    const before = byId.get(message.id)?.deliveries ?? []
    const deliveries = message.deliveries.map((delivery) => {
      const prior = before.find((item) => item.turn === delivery.turn)
      return prior &&
        FINISHED.includes(prior.status) &&
        !FINISHED.includes(delivery.status)
        ? prior
        : delivery
    })
    byId.set(message.id, { ...message, deliveries })
  }
  return [...byId.values()].sort((a, b) => a.sequence - b.sequence)
}

const ROUTING: Record<string, string> = {
  routing: 'Choosing an owner…',
  no_action: 'The coordinator found nothing for an agent to do.',
  failed: 'Routing failed. Mention an agent with @name to address it directly.',
}
const TASK_ROUTING: Record<string, string> = {
  routing: 'The orchestrator is reading this…',
}
const DELIVERY: Record<string, string> = {
  queued: 'queued',
  running: 'working…',
  waiting: 'waiting for a retry',
  delegated: 'waiting on delegated work',
  complete: 'replied',
  failed: 'failed',
  cancelled: 'cancelled',
}

/** A room is a channel: people and agents post, mentions deliver, the coordinator routes. */
export function Room({
  room,
  session,
}: {
  room: RoomSummary
  session: Session
}) {
  return (
    <RoomView
      roomId={room.id}
      session={session}
      label="Room"
      draftKey={`cubebot:${session.id}:room:${room.id}`}
      placeholder={`Message #${room.name} · @name addresses a member`}
      note="Each mentioned agent gets its own copy; without a mention the best-placed member answers."
      empty="Mention members with @name to ask each of them, or write without a mention and the coordinator picks one owner."
      mentionable={room.members}
    />
  )
}

/** A room's conversation, for a room in the sidebar and for a task's private one. */
export function RoomView({
  roomId,
  session,
  label,
  draftKey,
  placeholder,
  note,
  empty,
  mentionable,
  task = false,
}: {
  roomId: string
  session: Session
  label: string
  draftKey: string
  placeholder: string
  note: string
  empty: string
  /** Who `@` offers; every agent the viewer sees when absent. */
  mentionable?: { id: string; name: string }[]
  task?: boolean
}) {
  const [messages, setMessages] = useState<RoomMessage[]>([])
  const [loaded, setLoaded] = useState(false)
  const [readError, setReadError] = useState('')
  const scroller = useRef<HTMLDivElement>(null)
  const pinned = useRef(true)
  const agents = useAgents()
  const agentOf = (id: string) => agents.data?.find((agent) => agent.id === id)
  useEffect(
    () =>
      followWhileVisible(`/api/rooms/${roomId}/events`, (source) => {
        // History loads once the stream is following, so nothing falls between the two; a
        // reconnect reloads it to catch messages that changed while disconnected.
        source.onopen = () => {
          setReadError('')
          history((after) =>
            ok(
              api.GET('/api/rooms/{room}/messages', {
                params: { path: { room: roomId }, query: { after } },
              }),
            ),
          ).then(
            (loaded) => {
              setMessages((current) => merge(current, loaded))
              setLoaded(true)
            },
            (error) => setReadError(failure(error)),
          )
        }
        source.onerror = () =>
          setReadError(
            source.readyState === EventSource.CLOSED
              ? 'Live updates stopped. Reload the page to reconnect.'
              : 'Reconnecting to live updates…',
          )
        source.addEventListener('message', (event) => {
          const message: RoomMessage = JSON.parse(event.data)
          setMessages((current) => merge(current, [message]))
        })
      }),
    [roomId],
  )
  const last = messages.at(-1)?.sequence ?? 0
  useEffect(() => {
    if (loaded && last > 0) void markRead('rooms', roomId, last)
  }, [roomId, loaded, last])
  // Follow the newest message unless the reader scrolled up to read history.
  useEffect(() => {
    const element = scroller.current
    if (element && pinned.current) element.scrollTop = element.scrollHeight
  }, [loaded, messages])
  return (
    <section className="conversation" aria-label={label}>
      <div
        ref={scroller}
        className="transcript"
        tabIndex={0}
        aria-label="Room messages"
        onScroll={(event) => {
          const element = event.currentTarget
          pinned.current =
            element.scrollHeight - element.scrollTop - element.clientHeight < 80
        }}
      >
        {!loaded && <Skeleton lines={4} width="60%" />}
        {loaded && messages.length === 0 && (
          <Empty title="Nothing here yet.">{empty}</Empty>
        )}
        {(task
          ? rounds(messages)
          : messages.map((message) => ({
              message,
              deliveries: message.deliveries,
            }))
        ).map(({ message, deliveries }) => (
          <article className="exchange" key={message.id}>
            <div
              className={
                message.orchestrator || message.author_agent_id
                  ? 'msg msg--agent'
                  : 'msg msg--person'
              }
            >
              <AuthorAvatar message={message} agents={agents.data} />
              <div className="msg__body">
                <div className="msg__head">
                  <span className="msg__author">{message.author}</span>
                </div>
                <Markdown text={message.message} />
                {message.routing === 'continued' && (
                  <p className="small faint" role="status">
                    Coordinator added this to {message.note}’s open task as
                    steering.
                  </p>
                )}
                {(task ? TASK_ROUTING : ROUTING)[message.routing] && (
                  <p className="small faint" role="status">
                    {(task ? TASK_ROUTING : ROUTING)[message.routing]}
                  </p>
                )}
              </div>
            </div>
            {message.routing === 'clarify' && (
              <div className="msg msg--agent">
                <span className="avatar-initial msg__avatar" aria-hidden="true">
                  C
                </span>
                <div className="msg__body">
                  <div className="msg__head">
                    <span className="msg__author">Coordinator</span>
                  </div>
                  <Markdown text={message.note ?? ''} />
                </div>
              </div>
            )}
            {deliveries.map((item) => (
              <div className="msg msg--agent agent-message" key={item.turn}>
                <DeliveryAvatar
                  agent={agentOf(item.agent_id)}
                  name={item.agent}
                />
                <div className="msg__body">
                  <div className="msg__head">
                    <span className="msg__author">{item.agent}</span>
                    <span className="faint small">
                      · {DELIVERY[item.status] ?? item.status}
                    </span>
                  </div>
                  {item.terminal.length > 0 && (
                    <WorkLog
                      tools={item.terminal}
                      running={item.status === 'running'}
                    >
                      {item.terminal.map((tool) => (
                        <WorkCard key={tool.id} tool={tool} />
                      ))}
                    </WorkLog>
                  )}
                  <Markdown text={item.response ?? item.partial ?? ''} />
                  <Attachments turn={item.turn} items={item.attachments} />
                  {item.status === 'failed' && item.error && (
                    <p className="small" style={{ color: 'var(--danger)' }}>
                      {item.error.replaceAll('_', ' ')}
                    </p>
                  )}
                  <DeliveryActions
                    item={item}
                    agent={agentOf(item.agent_id)}
                    session={session}
                  />
                </div>
              </div>
            ))}
          </article>
        ))}
      </div>
      <Alert>{readError}</Alert>
      <Composer
        draftKey={draftKey}
        label="Message the room"
        placeholder={placeholder}
        note={note}
        mentionables={(mentionable ?? agents.data ?? []).map((member) => ({
          id: member.id,
          name: member.name,
          avatar: agentOf(member.id)?.avatar ?? null,
        }))}
        send={async (request_id, message) => {
          const posted = await ok(
            api.POST('/api/rooms/{room}/messages', {
              params: { path: { room: roomId } },
              body: { request_id, message },
            }),
          )
          setMessages((current) =>
            current.some((known) => known.id === posted.id)
              ? current
              : merge(current, [posted]),
          )
        }}
      />
    </section>
  )
}

/** A delivery's work is steered from here: the agent's own conversation never shows it. Needs
 *  send access on the agent, like the same actions in its conversation. */
function DeliveryActions({
  item,
  agent,
  session,
}: {
  item: RoomMessage['deliveries'][number]
  agent: AgentSummary | undefined
  session: Session
}) {
  const open = ['queued', 'running', 'delegated', 'waiting'].includes(
    item.status,
  )
  if (!agent || !open || !can(agent, 'conversation_send', session)) return null
  const turn = { id: item.turn, status: item.status }
  return (
    <div className="msg__actions">
      {item.status === 'waiting' && item.error !== 'budget_exhausted' && (
        <Retry conversation={item.agent_id} turn={turn} />
      )}
      <Stop
        conversation={item.agent_id}
        turn={turn}
        label={item.status === 'queued' ? 'Cancel' : 'Stop'}
      />
    </div>
  )
}

/** A person's picture, the orchestrator's block, or the block of the agent that wrote. */
function AuthorAvatar({
  message,
  agents,
}: {
  message: RoomMessage
  agents: AgentSummary[] | undefined
}) {
  if (message.orchestrator)
    return (
      <Cube avatar={ORCHESTRATOR_AVATAR} size={32} className="msg__avatar" />
    )
  if (message.author_agent_id)
    return (
      <DeliveryAvatar
        agent={agents?.find((agent) => agent.id === message.author_agent_id)}
        name={message.author}
      />
    )
  return <PersonAvatar login={message.author} className="msg__avatar" />
}

function DeliveryAvatar({
  agent,
  name,
}: {
  agent: ReturnType<typeof useAgents>['data'] extends (infer A)[] | undefined
    ? A | undefined
    : never
  name: string
}) {
  if (agent)
    return (
      <Cube
        agentId={agent.id}
        avatar={agent.avatar}
        mood={moodOf(agent)}
        size={32}
        className="msg__avatar"
      />
    )
  return (
    <span className="avatar-initial msg__avatar" aria-hidden="true">
      {name.slice(0, 1).toUpperCase()}
    </span>
  )
}
