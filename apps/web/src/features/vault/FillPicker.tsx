import { useEffect, useState } from 'react'
import type { VaultItem } from '../../api/client'
import type { components } from '../../api/schema'
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
    const [item, field] = choice.split(':') as [string, FillField]
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
        {items.flatMap((item) =>
          FIELDS[item.kind].map(([field, label]) => (
            <option key={`${item.id}:${field}`} value={`${item.id}:${field}`}>
              {item.name}: {label}
            </option>
          )),
        )}
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

type FillField = components['schemas']['VaultField']

const FIELDS: Record<VaultItem['kind'], [FillField, string][]> = {
  login: [
    ['username', 'username'],
    ['secret', 'password'],
  ],
  secret: [['secret', 'secret']],
  card: [
    ['holder', 'cardholder name'],
    ['number', 'card number'],
    ['expiry', 'expiry MM/YY'],
    ['expiry_month', 'expiry month'],
    ['expiry_year', 'expiry year'],
    ['cvc', 'security code'],
  ],
}
