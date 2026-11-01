import { useQuery, useQueryClient } from '@tanstack/react-query'
import { useState } from 'react'
import { admin } from '../../api/admin'
import { CAPABILITIES, type AgentSummary } from '../../api/types'
import { failure } from '../../api/client'
import { Alert, Field, Status, when } from '../../components/primitives'

/** Who may do what on this agent: grant, revoke by receipt, and the audit trail (ADR 0051). */
export function Access({ agent }: { agent: AgentSummary }) {
  const client = useQueryClient()
  const accounts = useQuery({ queryKey: ['accounts'], queryFn: admin.accounts })
  const teams = useQuery({ queryKey: ['teams'], queryFn: admin.teams })
  const events = useQuery({
    queryKey: ['access-events'],
    queryFn: () => admin.events(),
  })
  const [error, setError] = useState('')
  const [notice, setNotice] = useState('')
  const [busy, setBusy] = useState(false)
  const mine = (events.data ?? [])
    .filter(
      (event) =>
        event.kind.startsWith('access.') &&
        JSON.stringify(event.input).includes(agent.id),
    )
    .reverse()
  async function run(action: () => Promise<unknown>, done: string) {
    setBusy(true)
    setError('')
    setNotice('')
    try {
      await action()
      await client.invalidateQueries({ queryKey: ['access-events'] })
      setNotice(done)
    } catch (error) {
      setError(failure(error))
    } finally {
      setBusy(false)
    }
  }
  return (
    <>
      <section className="panel__section">
        <h2>Access</h2>
        <p className="panel__lead">
          Grants are exact: each gives one capability on this agent to one
          person or team. Administrators hold every capability through their
          role.
        </p>
        <form
          className="form"
          onSubmit={(event) => {
            event.preventDefault()
            const data = new FormData(event.currentTarget)
            const [kind, id] = String(data.get('recipient')).split(':') as [
              'user' | 'team',
              string,
            ]
            const capability = String(data.get('capability'))
            const conversational = capability.startsWith('conversation_')
            void run(
              () =>
                admin.grant(
                  { kind, id },
                  {
                    kind: conversational ? 'conversation' : 'agent',
                    id: agent.id,
                  },
                  capability,
                ),
              'Granted.',
            )
          }}
        >
          <Field label="Person or team">
            <select name="recipient" required defaultValue="">
              <option value="" disabled>
                Choose…
              </option>
              {accounts.data
                ?.filter((account) => account.active)
                .map((account) => (
                  <option key={account.id} value={`user:${account.id}`}>
                    {account.login}
                  </option>
                ))}
              {teams.data
                ?.filter((team) => team.active)
                .map((team) => (
                  <option key={team.id} value={`team:${team.id}`}>
                    Team: {team.name}
                  </option>
                ))}
            </select>
          </Field>
          <Field label="Capability">
            <select name="capability" defaultValue="conversation_send">
              {CAPABILITIES.filter(
                (capability) => capability !== 'room_manage',
              ).map((capability) => (
                <option key={capability} value={capability}>
                  {capability.replaceAll('_', ' ')}
                </option>
              ))}
            </select>
          </Field>
          <Alert>{error}</Alert>
          <Status>{notice}</Status>
          <div className="actions">
            <button className="btn btn--primary btn--sm" disabled={busy}>
              {busy ? 'Granting…' : 'Grant'}
            </button>
          </div>
        </form>
      </section>
      <section className="panel__section">
        <h3>Grant history</h3>
        <p className="panel__lead">
          Revoke a grant with its id and revision from the record below.
        </p>
        {mine.length === 0 && (
          <p className="small faint">No grants recorded for this agent yet.</p>
        )}
        <ul className="list">
          {mine.map((event) => (
            <li key={event.sequence} className="small">
              <div className="title">
                {event.kind.replace('access.', '')}
                <span className="faint" style={{ fontWeight: 400 }}>
                  · rev {event.revision} ·{' '}
                  {when(typeof event.at === 'string' ? event.at : undefined)}
                </span>
              </div>
              <pre className="small">
                {JSON.stringify(event.input, null, 1)}
              </pre>
              {event.kind === 'access.grant' && (
                <button
                  type="button"
                  className="link-btn small"
                  disabled={busy}
                  onClick={() =>
                    void run(
                      () =>
                        admin.revokeGrant(event.resource_id, event.revision),
                      'Revoked.',
                    )
                  }
                >
                  Revoke this grant
                </button>
              )}
            </li>
          ))}
        </ul>
      </section>
    </>
  )
}
