import { createFileRoute, Navigate } from '@tanstack/react-router'
import { notRetired, useAgents, usePreferences } from '../app/queries'
import { Cube } from '../components/Cube'
import { Sidebar } from '../features/shell/Sidebar'
import { useIsPhone } from '../features/shell/viewport'

export const Route = createFileRoute('/_app/')({ component: Home })

function Home() {
  const phone = useIsPhone()
  const agents = useAgents()
  const preferences = usePreferences()
  if (phone) return <Sidebar page />
  if (agents.isPending) return null
  const favorite = preferences.data?.favorites.find(
    (item) =>
      item.kind === 'agent' &&
      notRetired(agents.data).some((a) => a.id === item.id),
  )
  const first = favorite?.id ?? notRetired(agents.data)[0]?.id
  if (first)
    return (
      <Navigate to="/agents/$agentId" params={{ agentId: first }} replace />
    )
  return (
    <div className="empty shell-empty">
      <Cube size={96} follow mood="idle" />
      <h2>No agents yet.</h2>
      <p>
        An administrator creates the first agent from the sidebar. Members see
        agents once they are given access.
      </p>
    </div>
  )
}
