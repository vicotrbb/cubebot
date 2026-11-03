import { useEffect, useState } from 'react'
import { api, failure, ok, type VaultItem } from '../../api/client'
import type { AgentSummary } from '../../api/types'
import { Alert, Empty, when } from '../../components/primitives'
import { agentVault, ItemForm } from './ItemForm'

/** Logins and secrets the agent fills without reading them (docs/adr/identity.md#a-vault-the-desktop-arbiter-fills). */
export function Vault({
  agent,
  administrator,
}: {
  agent: AgentSummary
  administrator: boolean
}) {
  const [items, setItems] = useState<VaultItem[]>([])
  const [installation, setInstallation] = useState<VaultItem[]>([])
  const [editing, setEditing] = useState<VaultItem | 'new' | null>(null)
  const [error, setError] = useState('')
  async function load() {
    setItems(await agentVault(agent.id))
    if (administrator) setInstallation(await ok(api.GET('/api/vault')))
  }
  useEffect(() => {
    const failed = (error: unknown) => setError(failure(error))
    void agentVault(agent.id).then(setItems).catch(failed)
    if (administrator)
      void ok(api.GET('/api/vault')).then(setInstallation).catch(failed)
  }, [agent.id, administrator])
  async function remove(item: VaultItem) {
    setError('')
    try {
      await ok(
        api.DELETE('/api/vault/{item}', {
          params: { path: { item: item.id } },
        }),
      )
      await load()
    } catch (error) {
      setError(failure(error))
    }
  }
  const shown = (item: VaultItem) =>
    item.agent || !installation.some((other) => other.id === item.id)
  const list = (items: VaultItem[]) => (
    <ul className="list">
      {items.map((item) => (
        <li key={item.id}>
          <div className="title">
            {item.name}
            <span className="chip">{item.kind}</span>
          </div>
          {item.kind === 'login' && (
            <p className="small muted">
              {item.username} on {item.site}
            </p>
          )}
          <p className="small faint">
            {item.agent ? 'This agent only' : accessLabel(item)}
            {item.last_used_at
              ? `, last filled ${when(item.last_used_at)}`
              : ', never filled'}
          </p>
          <div className="actions" style={{ marginTop: 6 }}>
            <button
              type="button"
              className="link-btn small"
              onClick={() => setEditing(item)}
            >
              Edit
            </button>
            <button
              type="button"
              className="link-btn small"
              onClick={() => void remove(item)}
            >
              Delete
            </button>
          </div>
        </li>
      ))}
    </ul>
  )
  return (
    <>
      <section className="panel__section">
        <h2>Vault</h2>
        <p className="panel__lead">
          The agent types these into its desktop without seeing them. Secrets
          are never shown again after saving.
        </p>
        {!editing && (
          <div className="actions">
            <button
              type="button"
              className="btn btn--primary btn--sm"
              onClick={() => setEditing('new')}
            >
              New item
            </button>
          </div>
        )}
        {editing && (
          <ItemForm
            agent={agent}
            administrator={administrator}
            existing={editing === 'new' ? undefined : editing}
            onDone={() => {
              setEditing(null)
              void load().catch((error) => setError(failure(error)))
            }}
          />
        )}
        <Alert>{error}</Alert>
      </section>
      <section className="panel__section">
        <h3>Available to {agent.name}</h3>
        {items.length === 0 && <Empty title="No items yet." />}
        {list(items.filter(shown))}
      </section>
      {administrator && installation.length > 0 && (
        <section className="panel__section">
          <h3>Installation items</h3>
          {list(installation)}
        </section>
      )}
    </>
  )
}

function accessLabel(item: VaultItem) {
  return item.access.mode === 'all_agents'
    ? 'Installation, all agents'
    : `Installation, ${item.access.agents.length} agent(s)`
}
