import { useEffect, useRef, useState } from 'react'
import { api, failure, history, ok, type RoomMessage } from '../../api/client'
import type { RoomSummary, Session } from '../../api/types'
import { markRead, useAgents } from '../../app/queries'
import { followWhileVisible } from '../../app/stream'
import { Cube, moodOf } from '../../components/Cube'
import { PersonAvatar } from '../../components/PersonAvatar'
import { Alert, Empty, Skeleton } from '../../components/primitives'
import { Composer } from '../chat/Composer'
import '../chat/chat.css'

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
const DELIVERY: Record<string, string> = {
  queued: 'queued',
  running: 'working…',
  waiting: 'waiting for a retry in its conversation',
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
  const [messages, setMessages] = useState<RoomMessage[]>([])
  const [loaded, setLoaded] = useState(false)
  const [readError, setReadError] = useState('')
  const scroller = useRef<HTMLDivElement>(null)
  const pinned = useRef(true)
  const agents = useAgents()
  const agentOf = (id: string) => agents.data?.find((agent) => agent.id === id)
  useEffect(
    () =>
      followWhileVisible(`/api/rooms/${room.id}/events`, (source) => {
        // History loads once the stream is following, so nothing falls between the two; a
        // reconnect reloads it to catch messages that changed while disconnected.
        source.onopen = () => {
          setReadError('')
          history((after) =>
            ok(
              api.GET('/api/rooms/{room}/messages', {
                params: { path: { room: room.id }, query: { after } },
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
    [room.id],
  )
  const last = messages.at(-1)?.sequence ?? 0
  useEffect(() => {
    if (loaded && last > 0) void markRead('rooms', room.id, last)
  }, [room.id, loaded, last])
  // Follow the newest message unless the reader scrolled up to read history.
  useEffect(() => {
    const element = scroller.current
    if (element && pinned.current) element.scrollTop = element.scrollHeight
  }, [loaded, messages])
  return (
    <section className="conversation" aria-label="Room">
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
          <Empty title="Nothing here yet.">
            Mention members with @name to ask each of them, or write without a
            mention and the coordinator picks one owner.
          </Empty>
        )}
        {messages.map((message) => (
          <article className="exchange" key={message.id}>
            <div className="msg msg--person">
              <PersonAvatar login={message.author} className="msg__avatar" />
              <div className="msg__body">
                <div className="msg__head">
                  <span className="msg__author">{message.author}</span>
                </div>
                <p className="msg__text">{message.message}</p>
                {message.routing === 'continued' && (
                  <p className="small faint" role="status">
                    Coordinator added this to {message.note}’s open task as
                    steering.
                  </p>
                )}
                {ROUTING[message.routing] && (
                  <p className="small faint" role="status">
                    {ROUTING[message.routing]}
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
                  <p className="msg__text">{message.note}</p>
                </div>
              </div>
            )}
            {message.deliveries.map((item) => (
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
                  {item.response && (
                    <p className="msg__text">{item.response}</p>
                  )}
                  {item.status === 'failed' && item.error && (
                    <p className="small" style={{ color: 'var(--danger)' }}>
                      {item.error.replaceAll('_', ' ')}
                    </p>
                  )}
                </div>
              </div>
            ))}
          </article>
        ))}
      </div>
      <Alert>{readError}</Alert>
      <Composer
        draftKey={`cubebot:${session.id}:room:${room.id}`}
        label="Message the room"
        placeholder={`Message #${room.name} · @name addresses a member`}
        note="Each mentioned agent gets its own copy; without a mention the best-placed member answers."
        mentionables={room.members.map((member) => ({
          ...member,
          avatar: agentOf(member.id)?.avatar ?? null,
        }))}
        send={async (request_id, message) => {
          const posted = await ok(
            api.POST('/api/rooms/{room}/messages', {
              params: { path: { room: room.id } },
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
