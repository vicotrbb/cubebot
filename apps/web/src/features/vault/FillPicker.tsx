import { useEffect, useState } from 'react'
import type { VaultItem } from '../../api/client'
import type { AgentSummary } from '../../api/types'
import { agentVault, fillItem } from './ItemForm'

export function FillPicker({
  agent,
  holding,
}: {
  agent: AgentSummary
  holding: boolean
}) {
  const [items, setItems] = useState<VaultItem[]>([])
  const [choice, setChoice] = useState('')
  const [status, setStatus] = useState('')
  useEffect(() => {
    void agentVault(agent.id)
      .then(setItems)
      .catch(() => setItems([]))
  }, [agent.id])
  if (!holding || items.length === 0) return null
  async function fill() {
    const [item, field] = choice.split(':') as [string, 'username' | 'secret']
    setStatus('Filling…')
    const problem = await fillItem(agent.id, item, field)
    setStatus(problem ?? 'Filled.')
  }
  return (
    <div className="actions" style={{ padding: '10px 20px 0' }}>
      <select
        className="input"
        style={{ width: 'auto' }}
        aria-label="Vault item to fill"
        value={choice}
        onChange={(event) => setChoice(event.target.value)}
      >
        <option value="">Fill from vault…</option>
        {items.flatMap((item) => [
          ...(item.kind === 'login'
            ? [
                <option key={`${item.id}:u`} value={`${item.id}:username`}>
                  {item.name}: username
                </option>,
              ]
            : []),
          <option key={`${item.id}:s`} value={`${item.id}:secret`}>
            {item.name}: {item.kind === 'login' ? 'password' : 'secret'}
          </option>,
        ])}
      </select>
      <button
        type="button"
        className="btn btn--sm"
        disabled={!choice}
        onClick={() => void fill()}
      >
        Fill
      </button>
      {status && (
        <span className="muted small" role="status">
          {status}
        </span>
      )}
    </div>
  )
}
