import { createFileRoute, Link } from '@tanstack/react-router'
import { notRetired, useAgents, useStanding } from '../app/queries'
import { Cube } from '../components/Cube'
import { Home } from '../features/home/Home'

type View = 'list' | 'org'

export const Route = createFileRoute('/_app/')({
  validateSearch: (search: Record<string, unknown>): { view?: 'org' } =>
    search.view === 'org' ? { view: 'org' } : {},
  component: Landing,
})

function Landing() {
  const agents = useAgents()
  const standing = useStanding()
  const { view } = Route.useSearch()
  const navigate = Route.useNavigate()
  if (agents.isPending) return null
  if (agents.isSuccess && notRetired(agents.data).length === 0)
    return (
      <div className="empty shell-empty">
        <Cube size={96} follow mood="idle" />
        <h2>No agents yet.</h2>
        {standing.data?.can_create.agents ? (
          <>
            <p>Create one to start a conversation.</p>
            <Link to="/agents/new" className="btn btn--primary">
              Create agent
            </Link>
          </>
        ) : (
          <p>You see agents here once someone shares one with you.</p>
        )}
      </div>
    )
  return (
    <Home
      view={view ?? 'list'}
      onView={(next: View) =>
        navigate({
          search: next === 'org' ? { view: 'org' } : {},
          replace: true,
        })
      }
    />
  )
}
