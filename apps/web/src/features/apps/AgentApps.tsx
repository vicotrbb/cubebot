import type { AgentSummary } from '../../api/types'
import { Alert, Empty } from '../../components/primitives'
import { useApps } from './api'
import { AppGrid, GridSkeleton } from './Apps'
import './apps.css'

export function AgentApps({ agent }: { agent: AgentSummary }) {
  const apps = useApps()
  if (apps.isPending) return <GridSkeleton count={2} />
  if (apps.isError)
    return <Alert>The apps could not be read. Retry shortly.</Alert>
  const own = apps.data.filter((app) => app.agent_id === agent.id)
  if (own.length === 0)
    return (
      <Empty title="No apps yet">
        Describe one in {agent.name}&apos;s chat and it builds it here.
      </Empty>
    )
  return (
    <div className="apps apps--panel">
      <AppGrid apps={own} />
    </div>
  )
}
