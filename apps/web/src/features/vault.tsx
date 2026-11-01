import { useEffect, useState } from 'react'
import {
  type Agent,
  type VaultItem,
  api,
  ApiError,
  failure,
  ok,
} from '../api/client'

/** What the desktop can type (ADR 0086): printable ASCII and spaces. */
const TYPABLE = /^[\x20-\x7e]*$/

/** Logins and secrets the agent fills without reading them (ADR 0086). */
export function Vault({
  agent,
  administrator,
  onClose,
}: {
  agent: Agent
  administrator: boolean
  onClose: () => void
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
    <ul className="memory-list">
      {items.map((item) => (
        <li key={item.id} className="memory">
          <p>
            <strong>{item.name}</strong>{' '}
            {item.kind === 'login' && `${item.username} on ${item.site}`}
          </p>
          <div className="memory-meta">
            <span className="muted">
              {item.agent ? 'This agent only' : accessLabel(item)}
              {item.last_used_at
                ? `, last filled ${new Date(item.last_used_at).toLocaleString()}`
                : ', never filled'}
            </span>
            <button className="text-button" onClick={() => setEditing(item)}>
              Edit
            </button>
            <button className="text-button" onClick={() => void remove(item)}>
              Delete
            </button>
          </div>
        </li>
      ))}
    </ul>
  )
  return (
    <section>
      <header className="conversation-header">
        <div>
          <h1>{agent.name}: vault</h1>
          <p className="muted">
            The agent types these into its desktop without seeing them. Secrets
            are never shown again after saving.
          </p>
        </div>
        <div className="actions">
          <button onClick={() => setEditing('new')}>New item</button>
          <button className="secondary" onClick={onClose}>
            Back to conversation
          </button>
        </div>
      </header>
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
      {error && <p role="alert">{error}</p>}
      <h2>Available to {agent.name}</h2>
      {items.length === 0 && <p className="muted">No items yet.</p>}
      {list(items.filter(shown))}
      {administrator && installation.length > 0 && (
        <>
          <h2>Installation items</h2>
          {list(installation)}
        </>
      )}
    </section>
  )
}

function accessLabel(item: VaultItem) {
  return item.access.mode === 'all_agents'
    ? 'Installation, all agents'
    : `Installation, ${item.access.agents.length} agent(s)`
}

function agentVault(agent: string) {
  return ok(
    api.GET('/api/agents/{id}/vault', { params: { path: { id: agent } } }),
  )
}

/** Creates or edits an item; `fill` adds Save and fill, which types the secret for a person
 *  holding control. */
export function ItemForm({
  agent,
  administrator = false,
  existing,
  site,
  fill,
  onDone,
}: {
  agent: Agent
  administrator?: boolean
  existing?: VaultItem | undefined
  site?: string
  fill?: boolean
  onDone: () => void
}) {
  const [kind, setKind] = useState<Kind>(existing?.kind ?? 'login')
  const [error, setError] = useState('')
  const [busy, setBusy] = useState(false)
  async function save(form: HTMLFormElement, andFill: boolean) {
    const body = itemBody(new FormData(form), kind, agent.id, existing)
    if (!body) {
      setError(
        'The desktop types only printable ASCII and spaces; remove other characters.',
      )
      return
    }
    setBusy(true)
    setError('')
    try {
      const saved = await saveItem(existing, body)
      const problem = andFill && (await fillItem(agent.id, saved.id, 'secret'))
      if (problem) setError(`Saved, but not filled: ${problem}`)
      else onDone()
    } catch (error) {
      setError(failure(error))
    } finally {
      setBusy(false)
    }
  }
  return (
    <form
      onSubmit={(event) => {
        event.preventDefault()
        void save(event.currentTarget, false)
      }}
    >
      <label>
        Kind
        <select
          value={kind}
          onChange={(event) => setKind(event.target.value as Kind)}
        >
          <option value="login">Login</option>
          <option value="secret">Secret</option>
        </select>
      </label>
      <label>
        Name
        <input
          name="name"
          required
          maxLength={255}
          defaultValue={existing?.name ?? site ?? ''}
        />
      </label>
      {kind === 'login' && <LoginFields existing={existing} site={site} />}
      <label>
        {kind === 'login' ? 'Password' : 'Secret'}
        <input
          name="secret"
          type="password"
          required={!existing}
          maxLength={4096}
          autoComplete="new-password"
          placeholder={existing ? 'Leave empty to keep the current one' : ''}
        />
      </label>
      {!existing && administrator && <ScopeFields agent={agent} />}
      {error && <p role="alert">{error}</p>}
      <div className="actions">
        <button disabled={busy}>{busy ? 'Saving…' : 'Save'}</button>
        {fill && (
          <button
            type="button"
            disabled={busy}
            title="Saves, then types the password where the desktop's cursor is"
            onClick={(event) => {
              const form = event.currentTarget.form
              if (form?.reportValidity()) void save(form, true)
            }}
          >
            Save and fill
          </button>
        )}
        <button type="button" className="secondary" onClick={onDone}>
          Cancel
        </button>
      </div>
    </form>
  )
}

type Kind = VaultItem['kind']

/** The request for the form's values, or `null` when the desktop could not type them. */
function itemBody(
  data: FormData,
  kind: Kind,
  agent: string,
  existing: VaultItem | undefined,
) {
  const text = (name: string) => String(data.get(name) ?? '')
  const [secret, username] = [text('secret'), text('username')]
  if (!TYPABLE.test(secret) || !TYPABLE.test(username)) return null
  // An update keeps the item's scope and, without a choice here, its access.
  const installation = !existing && text('scope') === 'installation'
  const login = kind === 'login'
  const access =
    text('access') === 'all'
      ? { mode: 'all_agents' as const }
      : { mode: 'explicit_grants' as const, agents: [agent] }
  return {
    agent: existing || installation ? null : agent,
    kind,
    name: text('name'),
    site: login ? text('site') : null,
    username: login ? username : null,
    secret: secret || null,
    access: installation ? access : null,
  }
}

function saveItem(
  existing: VaultItem | undefined,
  body: NonNullable<ReturnType<typeof itemBody>>,
) {
  if (!existing) return ok(api.POST('/api/vault', { body }))
  return ok(
    api.PUT('/api/vault/{item}', {
      params: { path: { item: existing.id } },
      body,
    }),
  )
}

function LoginFields({
  existing,
  site,
}: {
  existing: VaultItem | undefined
  site: string | undefined
}) {
  return (
    <>
      <label>
        Site
        <input
          name="site"
          required
          maxLength={255}
          placeholder="github.com"
          defaultValue={existing?.site ?? site ?? ''}
        />
      </label>
      <label>
        Username
        <input
          name="username"
          required
          maxLength={255}
          autoComplete="off"
          defaultValue={existing?.username ?? ''}
        />
      </label>
    </>
  )
}

/** For administrators creating an item: the installation's, with its access. */
function ScopeFields({ agent }: { agent: Agent }) {
  return (
    <>
      <label>
        Scope
        <select name="scope" defaultValue="agent">
          <option value="agent">This agent only</option>
          <option value="installation">Installation</option>
        </select>
      </label>
      <label>
        Installation access
        <select name="access" defaultValue="all">
          <option value="all">All agents</option>
          <option value="this">Only {agent.name}</option>
        </select>
      </label>
    </>
  )
}

/** Types an item's field at the desktop's focus; the arbiter's refusal, if any. */
async function fillItem(
  agent: string,
  item: string,
  field: 'username' | 'secret',
): Promise<string | null> {
  const { response, error } = await api.POST(
    '/api/agents/{id}/vault/{item}/fill',
    { params: { path: { id: agent, item } }, body: { field } },
  )
  if (response.ok) return null
  const reason = (error as { error?: string } | undefined)?.error
  return reason ?? new ApiError(response.status).message
}

/** Fills a vault item into the desktop while the person holds control. */
export function FillPicker({
  agent,
  holding,
}: {
  agent: Agent
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
    <div className="actions">
      <select
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
      <button disabled={!choice} onClick={() => void fill()}>
        Fill
      </button>
      {status && (
        <span className="muted" role="status">
          {status}
        </span>
      )}
    </div>
  )
}
