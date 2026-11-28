import { useQuery, useQueryClient } from '@tanstack/react-query'
import { createFileRoute, Link } from '@tanstack/react-router'
import { useState } from 'react'
import { api, failure } from '../api/client'
import { useSession, useStanding } from '../app/queries'
import { Alert, confirmAction, Empty, Skeleton } from '../components/primitives'
import { answer, CONNECTIONS_KEY } from '../features/connections/api'
import { ConnectionsPage } from '../features/connections/ConnectionsPage'
import { Mark } from '../features/connections/Mark'
import '../features/connections/connections.css'
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
  if (!standing.data.can_create.connections) return <TurnedOff />
  return <ConnectionsPage personal />
}

/** Without the catalog, people still see and can remove the accounts they connected before. */
function TurnedOff() {
  const client = useQueryClient()
  const [error, setError] = useState('')
  const connections = useQuery({
    queryKey: CONNECTIONS_KEY,
    queryFn: () => answer(api.GET('/api/connections')),
  })
  const remove = async (id: string, slug: string) => {
    const yes = await confirmAction({
      title: `Remove ${slug}?`,
      body: 'Your agents lose its tools from their next call. An OAuth sign-in is revoked at the vendor.',
      action: 'Remove',
    })
    if (!yes) return
    setError('')
    try {
      await answer(
        api.DELETE('/api/connections/{id}', { params: { path: { id } } }),
      )
    } catch (failed) {
      setError(failure(failed))
    }
    await client.invalidateQueries({ queryKey: CONNECTIONS_KEY })
  }
  const mine = connections.data ?? []
  return (
    <>
      <Empty title="Connecting your own accounts is turned off.">
        An administrator turned it off for this installation. Ask them to turn
        it on, or to connect the account for everyone.
        {mine.length > 0 && ' Accounts you connected before keep working.'}
      </Empty>
      <Alert>{connections.error ? failure(connections.error) : error}</Alert>
      {mine.length > 0 && (
        <ul className="entries" aria-label="Your connected accounts">
          {mine.map((connection) => (
            <li className="entry" key={connection.id}>
              <Mark id={connection.connector} />
              <div className="entry__main">
                <span className="entry__name">{connection.slug}</span>
                <span className="entry__detail">{connection.account}</span>
              </div>
              <button
                type="button"
                className="btn btn--ghost btn--sm"
                onClick={() => void remove(connection.id, connection.slug)}
              >
                Remove
              </button>
            </li>
          ))}
        </ul>
      )}
    </>
  )
}
