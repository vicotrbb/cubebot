import { useQuery } from '@tanstack/react-query'
import { createFileRoute } from '@tanstack/react-router'
import { useState } from 'react'
import { admin } from '../api/admin'
import { CAPABILITIES } from '../api/types'
import { useAgents, useRooms } from '../app/queries'
import { Alert, Field, Skeleton, Status } from '../components/primitives'
import { useAdminAction } from '../features/platform/useAdminAction'
import '../features/platform/platform.css'

export const Route = createFileRoute('/_app/platform/teams')({
  component: Teams,
})

function Teams() {
  const teams = useQuery({ queryKey: ['teams'], queryFn: admin.teams })
  const accounts = useQuery({ queryKey: ['accounts'], queryFn: admin.accounts })
  const agents = useAgents()
  const rooms = useRooms()
  const { run, error, notice, busy } = useAdminAction([
    'teams',
    'access-events',
  ])
  const [explained, setExplained] = useState('')
  return (
    <div className="platform">
      <div>
        <p className="platform__lead">
          Teams group people so one grant reaches all of them. Grants are exact:
          one capability on one agent, room or conversation.
        </p>
        {teams.isPending && <Skeleton lines={3} />}
        <table className="table">
          <thead>
            <tr>
              <th>Team</th>
              <th>State</th>
              <th>Membership</th>
              <th />
            </tr>
          </thead>
          <tbody>
            {teams.data?.length === 0 && (
              <tr>
                <td colSpan={4} className="faint">
                  No teams yet.
                </td>
              </tr>
            )}
            {teams.data?.map((team) => (
              <tr key={team.id}>
                <td>{team.name}</td>
                <td>{team.active ? 'Active' : 'Deleted'}</td>
                <td>
                  {team.active && (
                    <form
                      className="actions"
                      onSubmit={(event) => {
                        event.preventDefault()
                        const data = new FormData(event.currentTarget)
                        void run(
                          () =>
                            admin.membership(
                              team.id,
                              String(data.get('account')),
                              team.revision,
                              data.get('present') === 'add',
                            ),
                          'Membership changed.',
                        )
                      }}
                    >
                      <select
                        className="input"
                        name="account"
                        required
                        style={{ width: 'auto', minHeight: 30 }}
                      >
                        {accounts.data
                          ?.filter((account) => account.active)
                          .map((account) => (
                            <option key={account.id} value={account.id}>
                              {account.login}
                            </option>
                          ))}
                      </select>
                      <select
                        className="input"
                        name="present"
                        style={{ width: 'auto', minHeight: 30 }}
                      >
                        <option value="add">add</option>
                        <option value="remove">remove</option>
                      </select>
                      <button className="btn btn--sm" disabled={busy}>
                        Apply
                      </button>
                    </form>
                  )}
                </td>
                <td className="num">
                  {team.active && (
                    <button
                      type="button"
                      className="link-btn small"
                      disabled={busy}
                      onClick={() => {
                        if (
                          window.confirm(
                            `Delete team ${team.name}? Its grants stop applying.`,
                          )
                        )
                          void run(
                            () => admin.deleteTeam(team.id, team.revision),
                            'Team deleted.',
                          )
                      }}
                    >
                      Delete
                    </button>
                  )}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      <aside className="platform__editor">
        <h2>New team</h2>
        <form
          className="form"
          onSubmit={(event) => {
            event.preventDefault()
            const form = event.currentTarget
            void run(
              () => admin.createTeam(String(new FormData(form).get('name'))),
              'Team created.',
            ).then(() => form.reset())
          }}
        >
          <Field label="Name">
            <input name="name" required maxLength={128} />
          </Field>
          <div className="actions">
            <button className="btn btn--primary btn--sm" disabled={busy}>
              Create team
            </button>
          </div>
        </form>
        <h2>Grant access</h2>
        <form
          className="form"
          onSubmit={(event) => {
            event.preventDefault()
            const data = new FormData(event.currentTarget)
            const [rk, rid] = String(data.get('recipient')).split(':') as [
              'user' | 'team',
              string,
            ]
            const [sk, sid] = String(data.get('scope')).split(':') as [
              'agent' | 'room',
              string,
            ]
            const capability = String(data.get('capability'))
            const scope =
              sk === 'agent' && capability.startsWith('conversation_')
                ? { kind: 'conversation' as const, id: sid }
                : { kind: sk, id: sid }
            void run(
              () => admin.grant({ kind: rk, id: rid }, scope, capability),
              'Granted.',
            )
          }}
        >
          <Field label="To">
            <select name="recipient" required defaultValue="">
              <option value="" disabled>
                Person or team…
              </option>
              {accounts.data
                ?.filter((a) => a.active)
                .map((a) => (
                  <option key={a.id} value={`user:${a.id}`}>
                    {a.login}
                  </option>
                ))}
              {teams.data
                ?.filter((t) => t.active)
                .map((t) => (
                  <option key={t.id} value={`team:${t.id}`}>
                    Team: {t.name}
                  </option>
                ))}
            </select>
          </Field>
          <Field label="On">
            <select name="scope" required defaultValue="">
              <option value="" disabled>
                Agent or room…
              </option>
              {agents.data?.map((agent) => (
                <option key={agent.id} value={`agent:${agent.id}`}>
                  {agent.name}
                </option>
              ))}
              {rooms.data?.map((room) => (
                <option key={room.id} value={`room:${room.id}`}>
                  Room: {room.name}
                </option>
              ))}
            </select>
          </Field>
          <Field label="Capability">
            <select name="capability" defaultValue="conversation_send">
              {CAPABILITIES.map((capability) => (
                <option key={capability} value={capability}>
                  {capability.replaceAll('_', ' ')}
                </option>
              ))}
            </select>
          </Field>
          <div className="actions">
            <button className="btn btn--sm" disabled={busy}>
              Grant
            </button>
          </div>
        </form>
        <h2>Explain my access</h2>
        <form
          className="form"
          onSubmit={(event) => {
            event.preventDefault()
            const data = new FormData(event.currentTarget)
            const [kind = '', id = ''] = String(data.get('scope')).split(':')
            void admin
              .explain({ kind, id }, String(data.get('capability')))
              .then((answer) =>
                setExplained(
                  `${answer.allowed ? 'Allowed' : 'Not allowed'}${answer.administrator ? ' as administrator' : ''}${
                    answer.grants.length
                      ? ` through ${answer.grants.length} grant(s)`
                      : ''
                  }.`,
                ),
              )
              .catch(() => setExplained('Could not explain.'))
          }}
        >
          <Field label="On">
            <select name="scope" required>
              {agents.data?.map((agent) => (
                <option key={agent.id} value={`agent:${agent.id}`}>
                  {agent.name}
                </option>
              ))}
            </select>
          </Field>
          <Field label="Capability">
            <select name="capability">
              {CAPABILITIES.map((c) => (
                <option key={c} value={c}>
                  {c.replaceAll('_', ' ')}
                </option>
              ))}
            </select>
          </Field>
          <div className="actions">
            <button className="btn btn--sm">Explain</button>
            {explained && <span className="small muted">{explained}</span>}
          </div>
        </form>
        <Alert>{error}</Alert>
        <Status>{notice}</Status>
      </aside>
    </div>
  )
}
