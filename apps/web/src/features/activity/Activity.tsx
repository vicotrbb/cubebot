import { useQuery, useQueryClient } from '@tanstack/react-query'
import { Link } from '@tanstack/react-router'
import {
  BellRing,
  BookOpen,
  CircleAlert,
  KeyRound,
  Mail,
  MessageCircleQuestion,
  PauseCircle,
  ShieldAlert,
  ShieldCheck,
  Video,
  Wallet,
  type LucideIcon,
} from 'lucide-react'
import { useEffect, useState, type ReactNode } from 'react'
import { api, failure, ok } from '../../api/client'
import type { components } from '../../api/schema'
import type { AgentSummary } from '../../api/types'
import {
  keys,
  markActivityRead,
  markRead,
  useActivity,
  useAgents,
  useUnreadReplies,
} from '../../app/queries'
import { usePush } from '../../app/push'
import { Cube } from '../../components/Cube'
import { Alert, Empty, Skeleton, when } from '../../components/primitives'
import { Request } from '../agent/CreationRequests'
import { byUrgency, excerpt, keyOf, needsYou, type Item } from './digest'
import '../agent/agent.css'
import './activity.css'

type Reply = components['schemas']['UnreadReply']
type Named = Map<string, AgentSummary>

const budget = (item: Item) => item.text.startsWith('Spent its budget')

const KINDS: Record<string, [string, LucideIcon]> = {
  watchdog: ['Watchdog', ShieldAlert],
  attention: ['Asks you', MessageCircleQuestion],
  approval_pending: ['Approval', ShieldCheck],
  login_pending: ['Log in', KeyRound],
  turn_waiting: ['Retry', PauseCircle],
  turn_failed: ['Failed', CircleAlert],
}

function destination(item: Item) {
  if (item.kind === 'watchdog') return '/watchdog'
  if (item.meeting) return `/meetings/${item.meeting}`
  if (item.task) return `/tasks/${item.task}`
  if (item.room) return `/rooms/${item.room}`
  if (!item.agent) return '/platform/people'
  if (item.kind === 'login_pending') return `/agents/${item.agent}/desktop`
  if (item.kind.endsWith('published_skill')) return '/skills'
  if (budget(item)) return `/agents/${item.agent}/usage`
  if (item.kind === 'attention' || !item.turn) return `/agents/${item.agent}`
  return `/agents/${item.agent}/thread/${item.turn}`
}

function useVisit(items: Item[] | undefined) {
  const [visit, setVisit] = useState<{ fresh: Set<string>; kept: Item[] }>()
  if (items && !visit)
    setVisit({
      fresh: new Set(items.filter((item) => item.unread).map(keyOf)),
      kept: items.filter((item) => !needsYou(item)),
    })
  const unread = items?.some((item) => item.unread)
  useEffect(() => {
    if (unread) void markActivityRead()
  }, [unread])
  const live = items ?? []
  const shown = new Set(live.map(keyOf))
  const updates = [
    ...live.filter((item) => !needsYou(item)),
    ...(visit?.kept ?? []).filter((item) => !shown.has(keyOf(item))),
  ].sort((a, b) => b.at.localeCompare(a.at))
  return {
    fresh: visit?.fresh ?? new Set<string>(),
    needs: byUrgency(live.filter(needsYou)),
    updates,
  }
}

export function Activity() {
  const activity = useActivity()
  const replies = useUnreadReplies()
  const agents = useAgents()
  const named: Named = new Map(agents.data?.map((agent) => [agent.id, agent]))
  const { fresh, needs, updates } = useVisit(activity.data)
  const wrote = replies.data ?? []
  const loading = activity.isPending || replies.isPending
  const quiet = !loading && !needs.length && !wrote.length && !updates.length
  return (
    <div className="page">
      <header className="page__head">
        <h1>
          Activity
          {!loading && !quiet && (
            <span className="faint"> · {summary(needs.length, wrote)}</span>
          )}
        </h1>
      </header>
      <div className="page__body page__body--narrow inbox">
        {loading && <Skeleton lines={6} />}
        {activity.isError && <Alert>{failure(activity.error)}</Alert>}
        {quiet && (
          <Empty title="You’re all caught up." action={<NotifyOffer />}>
            Questions, approvals and messages from your agents land here, the
            most pressing first.
          </Empty>
        )}
        {!loading && !quiet && (
          <>
            <NeedsYou items={needs} named={named} fresh={fresh} />
            <Messages replies={wrote} named={named} />
            {updates.length > 0 && (
              <Updates items={updates} named={named} fresh={fresh} />
            )}
          </>
        )}
      </div>
    </div>
  )
}

function summary(needs: number, wrote: Reply[]) {
  const parts = [
    needs
      ? `${needs} ${needs === 1 ? 'needs' : 'need'} you`
      : 'nothing needs you',
  ]
  if (wrote.length)
    parts.push(
      `${wrote.length} ${wrote.length === 1 ? 'agent' : 'agents'} wrote`,
    )
  return parts.join(' · ')
}

function NotifyOffer() {
  const push = usePush()
  if (push.state !== 'off') return null
  return (
    <button
      type="button"
      className="btn btn--ghost btn--sm"
      disabled={push.busy}
      title={
        push.error ||
        'Get a notification on this device when something lands here'
      }
      onClick={() => void push.enable()}
    >
      <BellRing /> {push.busy ? 'Turning on…' : 'Notify me'}
    </button>
  )
}

function Section({
  id,
  title,
  count,
  action,
  children,
}: {
  id: string
  title: string
  count: number
  action?: ReactNode
  children: ReactNode
}) {
  return (
    <section className="inbox__section" aria-labelledby={id}>
      <div className="inbox__head">
        <h2 id={id}>
          {title}
          {count > 0 && <span className="inbox__count">{count}</span>}
        </h2>
        {action}
      </div>
      {children}
    </section>
  )
}

function NeedsYou({
  items,
  named,
  fresh,
}: {
  items: Item[]
  named: Named
  fresh: Set<string>
}) {
  const approvals = [
    ...new Set(
      items
        .filter((item) => item.kind === 'approval_pending' && item.agent)
        .map((item) => item.agent as string),
    ),
  ]
  const rest = items.filter((item) => item.kind !== 'approval_pending')
  return (
    <Section
      id="needs-you"
      title="Needs you"
      count={items.length}
      action={<NotifyOffer />}
    >
      {items.length === 0 ? (
        <p className="inbox__quiet">
          Nothing is waiting on you. Questions and approvals show up here first.
        </p>
      ) : (
        <ul className="needs">
          {rest
            .filter((item) => item.kind === 'attention')
            .map((item) => (
              <Need key={keyOf(item)} item={item} named={named} fresh={fresh} />
            ))}
          {approvals.map((id) => {
            const agent = named.get(id)
            return agent ? <Approvals key={id} agent={agent} /> : null
          })}
          {rest
            .filter((item) => item.kind !== 'attention')
            .map((item) => (
              <Need key={keyOf(item)} item={item} named={named} fresh={fresh} />
            ))}
        </ul>
      )}
    </Section>
  )
}

function Face({
  agent,
  item,
}: {
  agent: AgentSummary | undefined
  item?: Item
}) {
  if (item?.kind === 'watchdog' && !item.agent)
    return (
      <span
        className="needs__face needs__face--installation"
        aria-hidden="true"
      >
        <ShieldAlert />
      </span>
    )
  return (
    <span className="needs__face" aria-hidden="true">
      <Cube agentId={agent?.id} avatar={agent?.avatar ?? null} size={30} />
    </span>
  )
}

function Need({
  item,
  named,
  fresh,
}: {
  item: Item
  named: Named
  fresh: Set<string>
}) {
  const client = useQueryClient()
  const [error, setError] = useState('')
  const [busy, setBusy] = useState(false)
  const agent = item.agent ? named.get(item.agent) : undefined
  const [label, Icon] = budget(item)
    ? (['Paused', Wallet] as const)
    : (KINDS[item.kind] ?? ['Asks you', CircleAlert])
  const to = destination(item)
  async function done() {
    setBusy(true)
    setError('')
    try {
      await ok(
        api.PUT('/api/activity/attention/{turn}', {
          params: { path: { turn: item.turn as string } },
        }),
      )
      await client.invalidateQueries({ queryKey: keys.activity })
    } catch (caught) {
      setError(failure(caught))
      setBusy(false)
    }
  }
  return (
    <li
      className="need"
      data-kind={item.kind}
      data-new={fresh.has(keyOf(item))}
    >
      <Face agent={agent} item={item} />
      <div className="need__body">
        <p className="need__head">
          <Link to={to} className="need__name">
            {agent?.name ?? (item.agent ? 'An agent' : 'Installation')}
          </Link>
          <span className="need__tag">
            <Icon aria-hidden="true" />
            {label}
          </span>
          <time dateTime={item.at}>{when(item.at)}</time>
        </p>
        <p className="need__text">{item.text}</p>
        <Alert>{error}</Alert>
      </div>
      <div className="need__actions">
        {item.kind === 'attention' && item.turn && (
          <button
            type="button"
            className="btn btn--ghost btn--sm"
            disabled={busy}
            onClick={() => void done()}
          >
            {busy ? 'Marking…' : 'Mark done'}
          </button>
        )}
        <Link
          to={to}
          className={
            item.kind === 'attention'
              ? 'btn btn--primary btn--sm'
              : 'btn btn--sm'
          }
        >
          {verb(item)}
        </Link>
      </div>
    </li>
  )
}

function verb(item: Item) {
  if (item.kind === 'watchdog') return 'Review'
  if (item.kind === 'attention') return 'Reply'
  if (item.kind === 'login_pending') return 'Open desktop'
  if (budget(item)) return 'See usage'
  return item.kind === 'turn_waiting' ? 'Review' : 'Open'
}

function Approvals({ agent }: { agent: AgentSummary }) {
  const client = useQueryClient()
  const [refused, setRefused] = useState('')
  const requests = useQuery({
    queryKey: ['creations', agent.id],
    queryFn: () =>
      ok(
        api.GET('/api/agents/{id}/creations', {
          params: { path: { id: agent.id } },
        }),
      ),
    staleTime: 0,
  })
  const settled = async (reason?: string) => {
    if (reason) setRefused(reason)
    await Promise.all([
      requests.refetch(),
      client.invalidateQueries({ queryKey: keys.activity }),
      client.invalidateQueries({ queryKey: keys.agents }),
    ])
  }
  const pending = requests.data ?? []
  return (
    <li className="need need--approvals" data-kind="approval_pending">
      <Face agent={agent} />
      <div className="need__body">
        <p className="need__head">
          <Link
            to="/agents/$agentId"
            params={{ agentId: agent.id }}
            className="need__name"
          >
            {agent.name}
          </Link>
          <span className="need__tag">
            <ShieldCheck aria-hidden="true" />
            Approval
          </span>
          {pending.length > 1 && (
            <span className="faint small">{pending.length} requests</span>
          )}
        </p>
        {requests.isPending && <Skeleton lines={2} />}
        {requests.isError && <Alert>{failure(requests.error)}</Alert>}
      </div>
      <div className="need__requests">
        {pending.map((request) => (
          <Request
            key={request.id}
            agent={agent}
            request={request}
            onSettled={(reason) => void settled(reason)}
          />
        ))}
        <Alert>{refused}</Alert>
      </div>
    </li>
  )
}

function Messages({ replies, named }: { replies: Reply[]; named: Named }) {
  const client = useQueryClient()
  const [busy, setBusy] = useState(false)
  async function readAll() {
    setBusy(true)
    await Promise.all(
      replies.map((reply) =>
        markRead(
          'conversations',
          reply.agent,
          Math.max(reply.sequence, named.get(reply.agent)?.last_sequence ?? 0),
        ),
      ),
    )
    client.setQueryData(['activity-replies'], [])
    await client.invalidateQueries({ queryKey: keys.agents })
    setBusy(false)
  }
  return (
    <Section
      id="from-agents"
      title="From your agents"
      count={replies.length}
      action={
        replies.length > 0 && (
          <button
            type="button"
            className="btn btn--ghost btn--sm"
            disabled={busy}
            onClick={() => void readAll()}
          >
            {busy ? 'Marking…' : 'Mark all read'}
          </button>
        )
      }
    >
      {replies.length === 0 ? (
        <p className="inbox__quiet">No unread messages.</p>
      ) : (
        <ul className="notes">
          {replies.map((reply) => {
            const agent = named.get(reply.agent)
            return (
              <li key={reply.agent}>
                <Link
                  to="/agents/$agentId"
                  params={{ agentId: reply.agent }}
                  className="note"
                >
                  <Cube
                    agentId={agent?.id}
                    avatar={agent?.avatar ?? null}
                    size={26}
                  />
                  <span className="note__who">
                    <span className="note__name">
                      {agent?.name ?? 'An agent'}
                    </span>
                    {reply.unread > 1 && (
                      <span
                        className="note__count"
                        aria-label={`${reply.unread} unread`}
                      >
                        {reply.unread}
                      </span>
                    )}
                  </span>
                  <span className="note__text">{excerpt(reply.excerpt)}</span>
                  <time dateTime={reply.at}>{when(reply.at)}</time>
                </Link>
              </li>
            )
          })}
        </ul>
      )}
    </Section>
  )
}

function Updates({
  items,
  named,
  fresh,
}: {
  items: Item[]
  named: Named
  fresh: Set<string>
}) {
  return (
    <Section id="updates" title="Updates" count={0}>
      <ul className="updates">
        {items.map((item) => {
          const Icon = UPDATE_ICONS[item.kind] ?? BookOpen
          const agent = item.agent ? named.get(item.agent) : undefined
          return (
            <li key={keyOf(item)} data-new={fresh.has(keyOf(item))}>
              <Link to={destination(item)} className="update">
                <span className="update__icon" aria-hidden="true">
                  <Icon />
                </span>
                <span className="update__text">
                  {agent && <strong>{agent.name} </strong>}
                  {agent ? lower(item.text) : item.text}
                </span>
                <time dateTime={item.at}>{when(item.at)}</time>
              </Link>
            </li>
          )
        })}
      </ul>
      {items.some((item) => item.kind.endsWith('published_skill')) && (
        <p className="inbox__quiet">
          New skills clear once you have seen them.
        </p>
      )}
    </Section>
  )
}

const UPDATE_ICONS: Record<string, LucideIcon> = {
  invitation_pending: Mail,
  recording_ready: Video,
  recording_failed: CircleAlert,
}

const lower = (text: string) => text.charAt(0).toLowerCase() + text.slice(1)
