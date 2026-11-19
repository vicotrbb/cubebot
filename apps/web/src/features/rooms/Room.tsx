import { useEffect, useRef, useState } from 'react'
import {
  api,
  failure,
  history,
  ok,
  resumeAfter,
  type RoomMessage,
} from '../../api/client'
import type { AgentSummary, RoomSummary, Session } from '../../api/types'
import { markRead, useAgents } from '../../app/queries'
import { followWhileVisible } from '../../app/stream'
import { Cube } from '../../components/Cube'
import { PersonAvatar } from '../../components/PersonAvatar'
import { Alert, Empty, Skeleton } from '../../components/primitives'
import { useFollowEnd } from '../chat/useFollowEnd'
import { Attachments } from '../chat/Attachments'
import { Composer } from '../chat/Composer'
import { Markdown, MentionNames } from '../chat/Markdown'
import type { Mentionable } from '../chat/MentionMenu'
import '../chat/chat.css'
import { ORCHESTRATOR_AVATAR } from '../tasks/orchestrator'
import { rounds } from './rounds'
import { AgentReply, DeliveryAvatar } from './Run'
import { runs } from './runs'
import './run.css'

const PAGE = 60
const FINISHED = ['complete', 'failed', 'cancelled']

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

const changing = (message: RoomMessage) =>
  message.routing === 'routing' ||
  message.deliveries.some((item) => !FINISHED.includes(item.status))

const ROUTING: Record<string, string> = {
  routing: 'Choosing an owner…',
  no_action: 'The coordinator found nothing for an agent to do.',
  failed: 'Routing failed. Mention an agent with @name to address it directly.',
}
const TASK_ROUTING: Record<string, string> = {
  routing: 'The orchestrator is reading this…',
}
function routingNote(message: RoomMessage, task: boolean) {
  if (message.note === 'no_agents')
    return 'No agent was in the room to read this.'
  return (task ? TASK_ROUTING : ROUTING)[message.routing]
}

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
      placeholder={`Message #${room.name}`}
      note="Mention several agents and one leads them as a team; without a mention the best-placed member answers."
      empty="Mention one agent with @name to ask it, several to have one lead them as a team, or write without a mention and the coordinator picks one owner."
      mentionable={room.members}
      people={room.people}
      closed={room.archived}
    />
  )
}

export function RoomView({
  roomId,
  session,
  label,
  draftKey,
  placeholder,
  note,
  empty,
  mentionable,
  people = [],
  task = false,
  closed = false,
}: {
  roomId: string
  session: Session
  label: string
  draftKey: string
  placeholder: string
  note: string
  empty: string
  mentionable?: { id: string; name: string }[]
  people?: { id: string; name: string }[]
  task?: boolean
  /** History only: the room takes no new messages. */
  closed?: boolean
}) {
  const [messages, setMessages] = useState<RoomMessage[]>([])
  const [loaded, setLoaded] = useState(false)
  const [readError, setReadError] = useState('')
  const [shown, setShown] = useState(PAGE)
  const scroller = useRef<HTMLDivElement>(null)
  const pinned = useRef(true)
  useFollowEnd(scroller, pinned)
  const agents = useAgents()
  const agentOf = (id: string) => agents.data?.find((agent) => agent.id === id)
  const mentionables: Mentionable[] = [
    ...(mentionable ?? agents.data ?? []).map((member) => ({
      id: member.id,
      name: member.name,
      avatar: agentOf(member.id)?.avatar ?? null,
    })),
    ...people.map((person) => ({ ...person, person: true })),
  ]
  const known = useRef(messages)
  useEffect(() => {
    known.current = messages
  }, [messages])
  useEffect(() => {
    let loading: AbortController | undefined
    // What the stream delivers first is no whole history, so only a finished load can resume.
    let whole = false
    const start = () => (whole ? resumeAfter(known.current, changing) : 0)
    // The stream replays from its cursor too, which by default is the room's first message.
    const url = () => `/api/rooms/${roomId}/events?after=${start()}`
    const stop = followWhileVisible(url, (source) => {
      // History loads once the stream is following, so nothing falls between the two.
      source.onopen = () => {
        setReadError('')
        loading?.abort()
        const { signal } = (loading = new AbortController())
        history(
          (after) =>
            ok(
              api.GET('/api/rooms/{room}/messages', {
                signal,
                params: { path: { room: roomId }, query: { after } },
              }),
            ),
          start(),
        ).then(
          (loaded) => {
            if (signal.aborted) return
            whole = true
            setMessages((current) => merge(current, loaded))
            setLoaded(true)
          },
          (error) => {
            if (!signal.aborted) setReadError(failure(error))
          },
        )
      }
      source.onerror = () => setReadError('Reconnecting to live updates…')
      source.addEventListener('message', (event) => {
        const message: RoomMessage = JSON.parse(event.data)
        setMessages((current) => merge(current, [message]))
      })
    })
    return () => {
      stop()
      loading?.abort()
    }
  }, [roomId])
  const last = messages.at(-1)?.sequence ?? 0
  useEffect(() => {
    if (loaded && last > 0) void markRead('rooms', roomId, last)
  }, [roomId, loaded, last])
  useEffect(() => {
    const element = scroller.current
    if (element && pinned.current) element.scrollTop = element.scrollHeight
  }, [loaded, messages])
  const run = runs(messages)
  const rows = task
    ? rounds(run.top)
    : run.top.map((message) => ({
        message,
        deliveries: message.deliveries,
      }))
  return (
    <section className="conversation" aria-label={label}>
      <MentionNames value={mentionables.map((member) => member.name)}>
        <div
          ref={scroller}
          className="transcript"
          tabIndex={0}
          aria-label="Room messages"
          onScroll={(event) => {
            const element = event.currentTarget
            pinned.current =
              element.scrollHeight - element.scrollTop - element.clientHeight <
              80
          }}
        >
          {!loaded && <Skeleton lines={4} width="60%" />}
          {loaded && messages.length === 0 && (
            <Empty title="Nothing here yet.">{empty}</Empty>
          )}
          {loaded && rows.length > shown && (
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
          {rows
            .slice(Math.max(0, rows.length - shown))
            .map(({ message, deliveries }) => (
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
                    <Attachments
                      turn={message.id}
                      items={message.attachments}
                    />
                    {message.routing === 'continued' && (
                      <p className="small faint" role="status">
                        Coordinator added this to {message.note}’s open task as
                        steering.
                      </p>
                    )}
                    {routingNote(message, task) && (
                      <p className="small faint" role="status">
                        {routingNote(message, task)}
                      </p>
                    )}
                  </div>
                </div>
                {message.routing === 'clarify' && (
                  <div className="msg msg--agent">
                    <span
                      className="avatar-initial msg__avatar"
                      aria-hidden="true"
                    >
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
                  <AgentReply
                    key={item.turn}
                    item={item}
                    run={run}
                    session={session}
                    agents={agents.data ?? []}
                  />
                ))}
              </article>
            ))}
        </div>
        <Alert>{readError}</Alert>
        {closed ? (
          <p className="room__closed small faint">
            This room was deleted. Its history stays for administrators; nobody
            can write here.
          </p>
        ) : (
          <Composer
            draftKey={draftKey}
            label="Message the room"
            placeholder={placeholder}
            note={note}
            mentionables={mentionables}
            attachable
            send={async (request_id, message, attachments) => {
              const posted = await ok(
                api.POST('/api/rooms/{room}/messages', {
                  params: { path: { room: roomId } },
                  body: { request_id, message, attachments },
                }),
              )
              setMessages((current) =>
                current.some((known) => known.id === posted.id)
                  ? current
                  : merge(current, [posted]),
              )
            }}
          />
        )}
      </MentionNames>
    </section>
  )
}

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
