import { useQuery, useQueryClient } from '@tanstack/react-query'
import { createFileRoute } from '@tanstack/react-router'
import { useState } from 'react'
import { api, failure, ok, type VaultItem } from '../api/client'
import { useAgents } from '../app/queries'
import { Alert, Skeleton, when } from '../components/primitives'
import { ItemForm } from '../features/vault/ItemForm'
import '../features/platform/platform.css'

export const Route = createFileRoute('/_app/platform/vault')({
  component: InstallationVault,
})

/** Installation logins and secrets with ADR 0081 access (ADR 0086). */
function InstallationVault() {
  const client = useQueryClient()
  const items = useQuery({
    queryKey: ['vault'],
    queryFn: () => ok(api.GET('/api/vault')),
  })
  const agents = useAgents()
  const [editing, setEditing] = useState<VaultItem | 'new' | null>(null)
  const [error, setError] = useState('')
  const first = agents.data?.[0]
  const refresh = () => client.invalidateQueries({ queryKey: ['vault'] })
  return (
    <div className="platform">
      <div>
        <p className="platform__lead">
          Items every agent, or only chosen agents, can fill into their desktops
          without seeing them. Agents' own items live on their Vault tabs.
        </p>
        {items.isPending && <Skeleton lines={3} />}
        <table className="table">
          <thead>
            <tr>
              <th>Name</th>
              <th>Kind</th>
              <th>Login</th>
              <th>Access</th>
              <th>Last filled</th>
              <th />
            </tr>
          </thead>
          <tbody>
            {items.data?.length === 0 && (
              <tr>
                <td colSpan={6} className="faint">
                  No installation items yet.
                </td>
              </tr>
            )}
            {items.data?.map((item) => (
              <tr key={item.id}>
                <td>{item.name}</td>
                <td>{item.kind}</td>
                <td>
                  {item.kind === 'login'
                    ? `${item.username} on ${item.site}`
                    : ''}
                </td>
                <td>
                  {item.access.mode === 'all_agents'
                    ? 'All agents'
                    : `${item.access.agents.length} agent(s)`}
                </td>
                <td>{item.last_used_at ? when(item.last_used_at) : 'never'}</td>
                <td className="num">
                  <button
                    type="button"
                    className="link-btn small"
                    onClick={() => setEditing(item)}
                  >
                    Edit
                  </button>{' '}
                  <button
                    type="button"
                    className="link-btn small"
                    onClick={() =>
                      void ok(
                        api.DELETE('/api/vault/{item}', {
                          params: { path: { item: item.id } },
                        }),
                      )
                        .then(refresh)
                        .catch((error) => setError(failure(error)))
                    }
                  >
                    Delete
                  </button>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
        <Alert>{error}</Alert>
      </div>
      <aside className="platform__editor">
        <h2>
          {editing && editing !== 'new' ? 'Edit item' : 'New installation item'}
        </h2>
        {first ? (
          <ItemForm
            key={editing === 'new' || !editing ? 'new' : editing.id}
            agent={first}
            administrator
            existing={editing && editing !== 'new' ? editing : undefined}
            onDone={() => {
              setEditing(null)
              void refresh()
            }}
          />
        ) : (
          <p className="small muted">
            Create an agent first; installation items name which agents may fill
            them.
          </p>
        )}
      </aside>
    </div>
  )
}
