import { useQuery, useQueryClient } from '@tanstack/react-query'
import {
  CreditCard,
  FileUp,
  KeyRound,
  LockKeyhole,
  MoreHorizontal,
  Pencil,
  Plus,
  Trash2,
} from 'lucide-react'
import { useState } from 'react'
import { api, failure, ok, type VaultItem } from '../../api/client'
import type { AgentSummary } from '../../api/types'
import { notRetired, useAgents } from '../../app/queries'
import {
  Alert,
  confirmAction,
  Dialog,
  Empty,
  Field,
  Menu,
  Skeleton,
  when,
} from '../../components/primitives'
import { ImportLogins } from './ImportLogins'
import { expired } from './cards'
import {
  CardFields,
  cardOf,
  LoginFields,
  SecretField,
  TYPABLE,
} from './ItemForm'
import './vault.css'

type Kind = VaultItem['kind']
type Access = VaultItem['access']

export function InstallationVault() {
  const client = useQueryClient()
  const items = useQuery({
    queryKey: ['vault'],
    queryFn: () => ok(api.GET('/api/vault')),
  })
  const [editing, setEditing] = useState<VaultItem | 'new' | null>(null)
  const [importing, setImporting] = useState(false)
  const [error, setError] = useState('')
  const refresh = () => client.invalidateQueries({ queryKey: ['vault'] })
  async function remove(item: VaultItem) {
    const yes = await confirmAction({
      title: `Delete ${item.name}?`,
      body: 'Agents can no longer fill it. This cannot be undone.',
      action: 'Delete',
    })
    if (!yes) return
    setError('')
    try {
      await ok(
        api.DELETE('/api/vault/{item}', {
          params: { path: { item: item.id } },
        }),
      )
      await refresh()
    } catch (error) {
      setError(failure(error))
    }
  }
  const newItem = (
    <div className="actions">
      <button type="button" className="btn" onClick={() => setImporting(true)}>
        <FileUp /> Import from browser
      </button>
      <button
        type="button"
        className="btn btn--primary"
        onClick={() => setEditing('new')}
      >
        <Plus /> New item
      </button>
    </div>
  )
  return (
    <div className="vault-page">
      <div className="vault-page__head">
        <p className="platform__lead">
          Logins, secrets and payment cards agents type into their desktops
          without seeing them. A card is typed only after a person approves each
          payment. Each agent's own items live on its Vault tab.
        </p>
        {Boolean(items.data?.length) && newItem}
      </div>
      {items.isPending && <Skeleton lines={3} />}
      <Alert>{items.error ? failure(items.error) : error}</Alert>
      {items.data?.length === 0 && (
        <Empty title="The installation vault is empty." action={newItem}>
          Save a login or secret once, or import the passwords your browser
          saved, choose which agents may use them, and they fill them on their
          desktops while the values stay hidden from them.
        </Empty>
      )}
      {Boolean(items.data?.length) && (
        <ul className="entries" aria-label="Installation items">
          {items.data?.map((item) => (
            <Row
              key={item.id}
              item={item}
              onEdit={() => setEditing(item)}
              onDelete={() => void remove(item)}
            />
          ))}
        </ul>
      )}
      <Dialog
        open={importing}
        title="Import browser passwords"
        onClose={() => setImporting(false)}
      >
        {importing && (
          <ImportLogins
            onDone={() => {
              setImporting(false)
              void refresh()
            }}
          />
        )}
      </Dialog>
      <Dialog
        open={editing !== null}
        title={
          editing && editing !== 'new' ? `Edit ${editing.name}` : 'New item'
        }
        onClose={() => setEditing(null)}
      >
        <InstallationItemForm
          existing={editing && editing !== 'new' ? editing : undefined}
          onDone={() => {
            setEditing(null)
            void refresh()
          }}
        />
      </Dialog>
    </div>
  )
}

function Row({
  item,
  onEdit,
  onDelete,
}: {
  item: VaultItem
  onEdit: () => void
  onDelete: () => void
}) {
  const Icon = ICONS[item.kind]
  return (
    <li className="entry">
      <span className="entry__icon" aria-hidden="true">
        <Icon />
      </span>
      <div className="entry__main">
        <span className="entry__name">{item.name}</span>
        <ItemDetail item={item} />
      </div>
      <div className="entry__meta">
        <span className="chip">{accessLabel(item.access)}</span>
        <span className="vault-filled">
          {item.last_used_at
            ? `Filled ${when(item.last_used_at)}`
            : 'Never filled'}
        </span>
        <Menu
          className="menu--right"
          trigger={<MoreHorizontal />}
          triggerClassName="btn btn--ghost btn--icon btn--sm"
          label={`Actions for ${item.name}`}
        >
          <button type="button" className="menu__item" onClick={onEdit}>
            <Pencil /> Edit
          </button>
          <button
            type="button"
            className="menu__item menu__item--danger"
            onClick={onDelete}
          >
            <Trash2 /> Delete
          </button>
        </Menu>
      </div>
    </li>
  )
}

const ICONS = { login: KeyRound, secret: LockKeyhole, card: CreditCard }

export function ItemDetail({ item }: { item: VaultItem }) {
  if (item.kind === 'login')
    return (
      <span className="entry__detail">
        {item.username} on {item.site}
      </span>
    )
  if (item.kind === 'secret')
    return <span className="entry__detail">Secret</span>
  const lapsed = expired(item.expires ?? '')
  return (
    <span className="entry__detail card-detail">
      <span>{item.brand}</span>
      <span className="tabular" aria-label={`ending ${item.last4}`}>
        •••• {item.last4}
      </span>
      <span className="tabular">
        {lapsed ? (
          <span className="chip chip--danger">Expired {item.expires}</span>
        ) : (
          `Expires ${item.expires}`
        )}
      </span>
    </span>
  )
}

function accessLabel(access: Access) {
  if (access.mode === 'all_agents') return 'All agents'
  const count = access.agents.length
  return count === 1 ? '1 agent' : `${count} agents`
}

function InstallationItemForm({
  existing,
  onDone,
}: {
  existing: VaultItem | undefined
  onDone: () => void
}) {
  const agents = useAgents()
  const [kind, setKind] = useState<Kind>(existing?.kind ?? 'login')
  const [access, setAccess] = useState<Access>(
    existing?.access ?? { mode: 'all_agents' },
  )
  const [error, setError] = useState('')
  const [busy, setBusy] = useState(false)
  async function save(form: HTMLFormElement) {
    const data = new FormData(form)
    const card = cardOf(data, kind, existing)
    if (typeof card === 'string') return setError(card)
    const text = (name: string) => String(data.get(name) ?? '')
    const [secret, username] = [text('secret'), text('username')]
    if (!TYPABLE.test(secret) || !TYPABLE.test(username))
      return setError(
        'The desktop types only printable ASCII and spaces; remove other characters.',
      )
    if (access.mode === 'explicit_grants' && access.agents.length === 0)
      return setError('Choose at least one agent, or allow all agents.')
    const login = kind === 'login'
    const body = {
      agent: null,
      kind,
      name: text('name'),
      site: login ? text('site') : null,
      username: login ? username : null,
      secret: secret || null,
      access,
      card,
    }
    setBusy(true)
    setError('')
    try {
      await (existing
        ? ok(
            api.PUT('/api/vault/{item}', {
              params: { path: { item: existing.id } },
              body,
            }),
          )
        : ok(api.POST('/api/vault', { body })))
      onDone()
    } catch (error) {
      setError(failure(error))
    } finally {
      setBusy(false)
    }
  }
  return (
    <form
      className="form"
      onSubmit={(event) => {
        event.preventDefault()
        void save(event.currentTarget)
      }}
    >
      <KindChoice value={kind} locked={Boolean(existing)} onPick={setKind} />
      <Field label="Name">
        <input
          name="name"
          required
          maxLength={255}
          placeholder={PLACEHOLDERS[kind]}
          defaultValue={existing?.name ?? ''}
        />
      </Field>
      {kind === 'login' && <LoginFields existing={existing} site={undefined} />}
      {kind === 'card' ? (
        <CardFields existing={existing} />
      ) : (
        <SecretField kind={kind} existing={existing} />
      )}
      <AccessChoice
        access={access}
        agents={notRetired(agents.data)}
        onChange={setAccess}
      />
      <Alert>{error}</Alert>
      <div className="actions">
        <button className="btn btn--primary" disabled={busy}>
          {busy ? 'Saving…' : existing ? 'Save changes' : 'Save item'}
        </button>
        <button type="button" className="btn btn--ghost" onClick={onDone}>
          Cancel
        </button>
      </div>
    </form>
  )
}

const PLACEHOLDERS: Record<Kind, string> = {
  login: 'GitHub',
  secret: 'Deploy token',
  card: 'Team virtual card',
}

function KindChoice({
  value,
  locked,
  onPick,
}: {
  value: Kind
  locked: boolean
  onPick: (kind: Kind) => void
}) {
  if (locked) return null
  return (
    <div className="tabs" role="radiogroup" aria-label="Kind">
      {(
        [
          ['login', 'Login'],
          ['secret', 'Secret'],
          ['card', 'Payment card'],
        ] as const
      ).map(([kind, label]) => (
        <button
          key={kind}
          type="button"
          role="radio"
          aria-checked={value === kind}
          onClick={() => onPick(kind)}
        >
          {label}
        </button>
      ))}
    </div>
  )
}

export function AccessChoice({
  access,
  agents,
  onChange,
  legend = 'Who may fill it',
}: {
  access: Access
  agents: AgentSummary[]
  onChange: (access: Access) => void
  legend?: string
}) {
  const chosen = access.mode === 'explicit_grants' ? access.agents : []
  const toggle = (id: string, on: boolean) =>
    onChange({
      mode: 'explicit_grants',
      agents: on ? [...chosen, id] : chosen.filter((agent) => agent !== id),
    })
  return (
    <fieldset className="vault-access">
      <legend>{legend}</legend>
      <label className="check">
        <input
          type="radio"
          name="access"
          checked={access.mode === 'all_agents'}
          onChange={() => onChange({ mode: 'all_agents' })}
        />
        Every agent, including ones created later
      </label>
      <label className="check">
        <input
          type="radio"
          name="access"
          checked={access.mode === 'explicit_grants'}
          disabled={agents.length === 0}
          onChange={() => onChange({ mode: 'explicit_grants', agents: chosen })}
        />
        Only the agents I choose
      </label>
      {access.mode === 'explicit_grants' && (
        <div className="vault-access__agents">
          {agents.map((agent) => (
            <label key={agent.id} className="check">
              <input
                type="checkbox"
                checked={chosen.includes(agent.id)}
                onChange={(event) => toggle(agent.id, event.target.checked)}
              />
              {agent.name}
            </label>
          ))}
        </div>
      )}
    </fieldset>
  )
}
