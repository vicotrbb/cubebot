import { createFileRoute, Link } from '@tanstack/react-router'
import { useSession, useStanding } from '../app/queries'
import { Empty, Skeleton } from '../components/primitives'
import { ConnectionsPage } from '../features/connections/ConnectionsPage'
import '../features/platform/platform.css'

export const Route = createFileRoute('/_app/connections')({
  head: () => ({ meta: [{ title: 'Your connections · Cubebot' }] }),
  component: YourConnections,
})

function YourConnections() {
  return (
    <div className="page">
      <header className="page__head">
        <h1>Your connections</h1>
      </header>
      <div className="page__body">
        <Body />
      </div>
    </div>
  )
}

function Body() {
  const session = useSession()
  const standing = useStanding()
  if (session.data?.administrator)
    return (
      <Empty
        title="Administrators connect accounts for the whole installation."
        action={
          <Link to="/platform/connections" className="btn">
            Open Platform connections
          </Link>
        }
      >
        Accounts you connect serve every eligible agent, so they live with the
        installation’s settings.
      </Empty>
    )
  if (!standing.data) return <Skeleton lines={4} />
  if (!standing.data.can_create.connections)
    return (
      <Empty title="Connecting your own accounts is turned off.">
        An administrator turned it off for this installation. Ask them to turn
        it on, or to connect the account for everyone.
      </Empty>
    )
  return <ConnectionsPage personal />
}
