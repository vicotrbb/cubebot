import { useEffect, useRef, useState } from 'react'
import { api, failure, ok, type RoomMessage } from '../../api/client'
import type { RoomSummary, Session } from '../../api/types'
import { markRead, useAgents } from '../../app/queries'
import { Cube, moodOf } from '../../components/Cube'
import { Alert, Empty, Skeleton } from '../../components/primitives'
import { Composer } from '../chat/Composer'
import '../chat/chat.css'

async function history(room: string) {
  const messages: RoomMessage[] = []
  for (;;) {
    const page = await ok(
      api.GET('/api/rooms/{room}/messages', {
        params: {
          path: { room },
          query: { after: messages.at(-1)?.sequence ?? 0 },
        },
      }),
    )
    messages.push(...page)
    if (page.length < 100) return messages
  }
}

/** Applies message snapshots by id, in room order. */
function merge(known: RoomMessage[], incoming: RoomMessage[]) {
  const byId = new Map(known.map((message) => [message.id, message]))
  for (const message of incoming) byId.set(message.id, message)
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
  const agents = useAgents()
  const agentOf = (id: string) => agents.data?.find((agent) => agent.id === id)
  useEffect(() => {
    const source = new EventSource(`/api/rooms/${room.id}/events`)
    // History loads once the stream is following, so nothing falls between the two; a
    // reconnect reloads it to catch messages that changed while disconnected.
    source.onopen = () => {
      setReadError('')
      history(room.id).then(
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
    return () => source.close()
  }, [room.id])
  const last = messages.at(-1)?.sequence ?? 0
  useEffect(() => {
    if (loaded && last > 0) void markRead('rooms', room.id, last)
    const element = scroller.current
    if (element) element.scrollTop = element.scrollHeight
  }, [room.id, loaded, last, messages])
  return (
    <section className="conversation" aria-label="Room">
      <div
        ref={scroller}
        className="transcript"
        tabIndex={0}
        aria-label="Room messages"
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
              <span className="avatar-initial msg__avatar" aria-hidden="true">
                {message.author.slice(0, 1).toUpperCase()}
              </span>
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
        note="Each mentioned agent gets its own copy; without a mention one owner is chosen."
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
