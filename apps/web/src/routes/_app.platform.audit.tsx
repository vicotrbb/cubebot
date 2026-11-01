import { useQuery } from '@tanstack/react-query'
import { createFileRoute } from '@tanstack/react-router'
import { useState } from 'react'
import { admin } from '../api/admin'
import { Alert, Skeleton } from '../components/primitives'
import '../features/platform/platform.css'

export const Route = createFileRoute('/_app/platform/audit')({
  component: Audit,
})

/** Administrator-visible identity and access mutations, by sequence. */
function Audit() {
  const [after, setAfter] = useState(0)
  const events = useQuery({
    queryKey: ['access-events', after],
    queryFn: () => admin.events(after),
  })
  const [open, setOpen] = useState<number | null>(null)
  return (
    <div>
      <p className="platform__lead">
        Every account, invitation, team, grant and sign-in binding change, with
        its actor and the non-secret request. Passwords and tokens never appear
        here.
      </p>
      {events.isPending && <Skeleton lines={4} />}
      {events.isError && <Alert>The audit log could not be loaded.</Alert>}
      <table className="table">
        <thead>
          <tr>
            <th className="num">#</th>
            <th>Kind</th>
            <th>Actor</th>
            <th>Resource</th>
            <th className="num">Rev</th>
            <th />
          </tr>
        </thead>
        <tbody>
          {events.data?.map((event) => (
            <tr
              key={event.sequence}
              className={open === event.sequence ? 'selected' : undefined}
            >
              <td className="num">{event.sequence}</td>
              <td>{event.kind}</td>
              <td>
                <code>{event.actor_id.slice(0, 8)}</code>
              </td>
              <td>
                <code>{event.resource_id.slice(0, 8)}</code>
                {open === event.sequence && (
                  <pre className="small">
                    {JSON.stringify(event.input, null, 1)}
                  </pre>
                )}
              </td>
              <td className="num">{event.revision}</td>
              <td className="num">
                <button
                  type="button"
                  className="link-btn small"
                  onClick={() =>
                    setOpen(open === event.sequence ? null : event.sequence)
                  }
                >
                  {open === event.sequence ? 'Hide' : 'Request'}
                </button>
              </td>
            </tr>
          ))}
        </tbody>
      </table>
      {events.data && events.data.length === 100 && (
        <div className="actions" style={{ marginTop: 12 }}>
          <button
            type="button"
            className="btn btn--sm"
            onClick={() => setAfter(events.data.at(-1)?.sequence ?? after)}
          >
            Next 100
          </button>
        </div>
      )}
    </div>
  )
}
