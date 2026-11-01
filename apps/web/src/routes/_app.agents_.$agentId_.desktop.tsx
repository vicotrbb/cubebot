import { createFileRoute } from '@tanstack/react-router'
import { can } from '../api/types'
import { useAgent, useSession } from '../app/queries'
import { Empty, Skeleton } from '../components/primitives'
import { DesktopView } from '../features/desktop/DesktopView'
import '../features/agent/agent.css'

export const Route = createFileRoute('/_app/agents_/$agentId_/desktop')({
  validateSearch: (search: Record<string, unknown>): { take?: true } =>
    search.take === true || search.take === 'true' ? { take: true } : {},
  component: DesktopRoute,
})

function DesktopRoute() {
  const { agentId } = Route.useParams()
  const { take } = Route.useSearch()
  const session = useSession()
  const { agent, isPending } = useAgent(agentId)
  if (isPending || !session.data)
    return (
      <div className="page__body">
        <Skeleton lines={2} width="40%" />
      </div>
    )
  if (!agent || !can(agent, 'browser_view', session.data))
    return (
      <Empty title="This desktop is unavailable.">
        You need browser access on this agent.
      </Empty>
    )
  return (
    <DesktopView
      key={agent.id}
      agent={agent}
      accountId={session.data.id}
      take={take}
      canControl={can(agent, 'browser_control', session.data)}
    />
  )
}
