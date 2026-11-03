import { createFileRoute, Link } from '@tanstack/react-router'
import {
  BookOpen,
  CircleAlert,
  KeyRound,
  Mail,
  PauseCircle,
} from 'lucide-react'
import { useActivity, useAgents } from '../app/queries'
import { Empty, Skeleton, when } from '../components/primitives'

export const Route = createFileRoute('/_app/activity')({ component: Activity })

const ICONS: Record<string, React.ComponentType<{ className?: string }>> = {
  login_pending: KeyRound,
  turn_waiting: PauseCircle,
  turn_failed: CircleAlert,
  invitation_pending: Mail,
  dream_published_skill: BookOpen,
}

/** What needs the person: pending logins, waiting or failed work, invitations, new skills. */
function Activity() {
  const activity = useActivity()
  const agents = useAgents()
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
            const to = item.agent
              ? item.kind === 'login_pending'
                ? `/agents/${item.agent}/desktop`
                : item.turn
                  ? `/agents/${item.agent}/thread/${item.turn}`
                  : `/agents/${item.agent}`
              : '/platform/people'
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
