import { createFileRoute, Link } from '@tanstack/react-router'
import {
  BookOpen,
  CircleAlert,
  KeyRound,
  Mail,
  PauseCircle,
} from 'lucide-react'
import { useEffect } from 'react'
import { markActivityRead, useActivity, useAgents } from '../app/queries'
import type { components } from '../api/schema'
import { Empty, Skeleton, when } from '../components/primitives'

export const Route = createFileRoute('/_app/activity')({ component: Activity })

const ICONS: Record<string, React.ComponentType<{ className?: string }>> = {
  login_pending: KeyRound,
  turn_waiting: PauseCircle,
  turn_failed: CircleAlert,
  invitation_pending: Mail,
  dream_published_skill: BookOpen,
  agent_published_skill: BookOpen,
}

function destination(item: components['schemas']['ActivityItem']) {
  if (item.task) return `/tasks/${item.task}`
  if (item.room) return `/rooms/${item.room}`
  if (!item.agent) return '/platform/people'
  if (item.kind === 'login_pending') return `/agents/${item.agent}/desktop`
  if (item.kind.endsWith('published_skill')) return '/skills'
  if (item.text.endsWith('work is paused')) return `/agents/${item.agent}/usage`
  return item.turn
    ? `/agents/${item.agent}/thread/${item.turn}`
    : `/agents/${item.agent}`
}

function Activity() {
  const activity = useActivity()
  const agents = useAgents()
  const fresh = activity.data?.some((item) => item.unread)
  useEffect(() => {
    if (fresh) void markActivityRead()
  }, [fresh])
  const name = (id?: string | null) =>
    agents.data?.find((agent) => agent.id === id)?.name
  return (
    <div className="page">
      <header className="page__head">
        <h1>Activity</h1>
      </header>
      <div className="page__body page__body--list">
        {activity.isPending && <Skeleton lines={4} />}
        {activity.data?.length === 0 && (
          <Empty title="Nothing needs you.">
            Login requests, work waiting for a retry, failures and new skills
            show up here.
          </Empty>
        )}
        <ul className="list">
          {activity.data?.map((item, index) => {
            const Icon = ICONS[item.kind] ?? CircleAlert
            const to = destination(item)
            return (
              <li key={`${item.kind}:${item.turn ?? item.agent ?? index}`}>
                <div className="title">
                  <Icon className="row__icon" />
                  <Link to={to}>{item.text}</Link>
                </div>
                <p className="small faint">
                  {name(item.agent) ?? ''} {name(item.agent) ? '·' : ''}{' '}
                  {when(item.at)}
                </p>
              </li>
            )
          })}
        </ul>
      </div>
    </div>
  )
}
