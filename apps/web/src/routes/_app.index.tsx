import { createFileRoute, Link, Navigate } from '@tanstack/react-router'
import {
  notRetired,
  useAgents,
  usePreferences,
  useSession,
} from '../app/queries'
import { Cube } from '../components/Cube'
import { Sidebar } from '../features/shell/Sidebar'
import { useIsPhone } from '../features/shell/viewport'

export const Route = createFileRoute('/_app/')({ component: Home })

function Home() {
  const phone = useIsPhone()
  const agents = useAgents()
  const preferences = usePreferences()
  const session = useSession()
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
      {session.data?.administrator ? (
        <>
          <p>Create one to start a conversation.</p>
          <Link to="/agents/new" className="btn btn--primary">
            Create agent
          </Link>
        </>
      ) : (
        <p>You see agents here once an administrator gives you access.</p>
      )}
    </div>
  )
}
