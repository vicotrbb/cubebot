import { Attachments } from '../chat/Attachments'
import { Markdown } from '../chat/Markdown'
import { useQueryClient } from '@tanstack/react-query'
import { Link } from '@tanstack/react-router'
import { ChevronLeft, Megaphone } from 'lucide-react'
import { useEffect, useRef, useState } from 'react'
import { api, ok } from '../../api/client'
import type { AgentSummary, Session } from '../../api/types'
import { keys, notRetired, useAgents, useBroadcasts } from '../../app/queries'
import { Cube, moodOf } from '../../components/Cube'
import { PersonAvatar } from '../../components/PersonAvatar'
import { Alert, Empty, Skeleton, when } from '../../components/primitives'
import { Composer } from '../chat/Composer'
import { useIsPhone } from '../shell/viewport'
import '../agent/agent.css'
import '../chat/chat.css'
import './broadcast.css'

type Feed = NonNullable<ReturnType<typeof useBroadcasts>['data']>
type Message = Feed['messages'][number]
type Delivery = Message['deliveries'][number]

const FINISHED = ['complete', 'failed', 'cancelled']
const STATUS: Record<string, string> = {
  queued: 'queued',
  running: 'working…',
  waiting: 'waiting for a retry',
  delegated: 'waiting on delegated work',
  complete: 'replied',
  failed: 'failed',
  cancelled: 'cancelled',
}

export function Broadcast({ session }: { session: Session }) {
  const feed = useBroadcasts(true, true)
  const agents = useAgents()
  const client = useQueryClient()
  const scroller = useRef<HTMLDivElement>(null)
  const pinned = useRef(true)
  const messages = feed.data?.messages
  const live = notRetired(agents.data)
  useMarkRead(feed.data)
  useEffect(() => {
    const element = scroller.current
    if (element && pinned.current) element.scrollTop = element.scrollHeight
  }, [messages])
  return (
    <div className="agent-screen">
      <Head live={live} />
      <section className="conversation" aria-label="Broadcast">
        <div
          ref={scroller}
          className="transcript"
          tabIndex={0}
          aria-label="Broadcasts"
          onScroll={(event) => {
            const element = event.currentTarget
            pinned.current =
              element.scrollHeight - element.scrollTop - element.clientHeight <
              80
          }}
        >
          {feed.isPending && <Skeleton lines={4} width="60%" />}
          {messages?.length === 0 && (
            <Empty title="Nothing broadcast yet.">
              A message here reaches every agent at once, each in its own
              conversation, and their replies gather under it. Agents you allow
              in their settings can broadcast here too.
            </Empty>
          )}
          {messages?.map((message) => (
            <BroadcastMessage
              key={message.id}
              message={message}
              agents={agents.data ?? []}
            />
          ))}
        </div>
        <Alert>{feed.isError ? 'Broadcasts could not be loaded.' : ''}</Alert>
        <Composer
          draftKey={`cubebot:${session.id}:broadcast`}
          label="Message every agent"
          placeholder="Message every agent"
          note={`Reaches ${live.length} ${live.length === 1 ? 'agent' : 'agents'}, each in its own conversation; stopped agents start to answer.`}
          send={async (request_id, message) => {
            await ok(
              api.POST('/api/broadcasts', { body: { request_id, message } }),
            )
            pinned.current = true
            await client.invalidateQueries({ queryKey: keys.broadcasts })
          }}
        />
      </section>
    </div>
  )
}

function useMarkRead(feed: Feed | undefined) {
  const client = useQueryClient()
  const last = feed?.messages.at(-1)?.sequence ?? 0
  const read = feed?.read_sequence ?? 0
  useEffect(() => {
    if (last <= read) return
    void api
      .PUT('/api/broadcasts/read', { body: { sequence: last } })
      .then(({ response }) => {
        if (!response.ok) return
        client.setQueryData<Feed>(keys.broadcasts, (current) =>
          current ? { ...current, read_sequence: last } : current,
        )
      })
      .catch(() => undefined)
  }, [client, last, read])
}

function Head({ live }: { live: AgentSummary[] }) {
  const phone = useIsPhone()
  const senders = live.filter((agent) => agent.broadcast).length
  return (
    <header className="agent-head">
      {phone && (
        <Link
          to="/"
          className="btn btn--ghost btn--icon"
          aria-label="Back to home"
        >
          <ChevronLeft />
        </Link>
      )}
      <span className="broadcast-mark" aria-hidden="true">
        <Megaphone />
      </span>
      <div className="agent-head__title">
        <h1>Broadcast</h1>
        <p className="faint small">
          Every agent hears what is posted here · {live.length}{' '}
          {live.length === 1 ? 'agent' : 'agents'}
          {senders > 0 && ` · ${senders} can broadcast`}
        </p>
      </div>
    </header>
  )
}

function BroadcastMessage({
  message,
  agents,
}: {
  message: Message
  agents: AgentSummary[]
}) {
  const [open, setOpen] = useState(false)
  const sender = agents.find((agent) => agent.id === message.agent)
  const replied = message.deliveries.filter(
    (delivery) => delivery.status === 'complete',
  ).length
  const working = message.deliveries.filter(
    (delivery) => !FINISHED.includes(delivery.status),
  ).length
  const failed = message.deliveries.length - replied - working
  return (
    <article className="exchange">
      <div className={message.agent ? 'msg msg--agent' : 'msg msg--person'}>
        {message.agent ? (
          <Cube
            agentId={sender?.id}
            avatar={sender?.avatar ?? null}
            mood={sender ? moodOf(sender) : 'idle'}
            size={32}
            className="msg__avatar"
          />
        ) : (
          <PersonAvatar
            account={message.account}
            login={message.author}
            className="msg__avatar"
          />
        )}
        <div className="msg__body">
          <div className="msg__head">
            <span className="msg__author">{message.author}</span>
            {message.agent && <span className="chip">Agent</span>}
            <time className="msg__time" dateTime={message.at}>
              {when(message.at)}
            </time>
          </div>
          <Markdown text={message.message} />
          {message.deliveries.length > 0 && (
            <button
              type="button"
              className="thread-line"
              aria-expanded={open}
              onClick={() => setOpen(!open)}
            >
              <span className="broadcast-faces" aria-hidden="true">
                {message.deliveries.slice(0, 4).map((delivery) => (
                  <Face
                    key={delivery.turn}
                    delivery={delivery}
                    agents={agents}
                  />
                ))}
              </span>
              {summary(message.deliveries.length, replied, working, failed)}
              <span className="faint">{open ? 'Hide' : 'Replies'}</span>
            </button>
          )}
          {message.deliveries.length === 0 && (
            <p className="small faint">No other agent was there to hear it.</p>
          )}
        </div>
      </div>
      {open && (
        <ul className="broadcast-replies">
          {message.deliveries.map((delivery) => (
            <Reply key={delivery.turn} delivery={delivery} agents={agents} />
          ))}
        </ul>
      )}
    </article>
  )
}

function summary(
  total: number,
  replied: number,
  working: number,
  failed: number,
) {
  const parts = [`${total} ${total === 1 ? 'agent' : 'agents'}`]
  if (replied) parts.push(`${replied} replied`)
  if (working) parts.push(`${working} working`)
  if (failed) parts.push(`${failed} failed`)
  return parts.join(' · ')
}

function Face({
  delivery,
  agents,
}: {
  delivery: Delivery
  agents: AgentSummary[]
}) {
  const agent = agents.find((item) => item.id === delivery.agent_id)
  return (
    <Cube
      agentId={agent?.id}
      avatar={agent?.avatar ?? null}
      mood="idle"
      size={18}
    />
  )
}

function Reply({
  delivery,
  agents,
}: {
  delivery: Delivery
  agents: AgentSummary[]
}) {
  const agent = agents.find((item) => item.id === delivery.agent_id)
  return (
    <li className="msg msg--agent">
      <Cube
        agentId={agent?.id}
        avatar={agent?.avatar ?? null}
        mood={agent ? moodOf(agent) : 'idle'}
        size={28}
        className="msg__avatar"
      />
      <div className="msg__body">
        <div className="msg__head">
          <Link
            className="msg__author"
            to="/agents/$agentId"
            params={{ agentId: delivery.agent_id }}
          >
            {delivery.agent}
          </Link>
          <span className="faint small" data-status={delivery.status}>
            {STATUS[delivery.status] ?? delivery.status}
          </span>
        </div>
        {delivery.response && <Markdown text={delivery.response} />}
        <Attachments turn={delivery.turn} items={delivery.attachments} />
        {delivery.status === 'failed' && delivery.error && (
          <p className="small" role="status" style={{ color: 'var(--danger)' }}>
            {delivery.error.replaceAll('_', ' ')}
          </p>
        )}
      </div>
    </li>
  )
}
