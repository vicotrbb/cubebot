import { useState } from 'react'
import { api, ApiError, failure, ok, type VaultItem } from '../../api/client'
import type { AgentSummary } from '../../api/types'
import { Alert, Field } from '../../components/primitives'

/** What the desktop can type: printable ASCII and spaces. */
export const TYPABLE = /^[\x20-\x7e]*$/
type Kind = VaultItem['kind']

export function ItemForm({
  agent,
  administrator = false,
  existing,
  site,
  fill,
  onDone,
}: {
  agent: AgentSummary
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
      className="form"
      onSubmit={(event) => {
        event.preventDefault()
        void save(event.currentTarget, false)
      }}
    >
      <div className="row">
        <Field label="Kind">
          <select
            value={kind}
            onChange={(event) => setKind(event.target.value as Kind)}
          >
            <option value="login">Login</option>
            <option value="secret">Secret</option>
          </select>
        </Field>
        <Field label="Name">
          <input
            name="name"
            required
            maxLength={255}
            defaultValue={existing?.name ?? site ?? ''}
          />
        </Field>
      </div>
      {kind === 'login' && <LoginFields existing={existing} site={site} />}
      <Field
        label={kind === 'login' ? 'Password' : 'Secret'}
        hint={
          existing ? 'Leave empty to keep the current one' : 'Never shown again'
        }
      >
        <input
          name="secret"
          type="password"
          required={!existing}
          maxLength={4096}
          autoComplete="new-password"
        />
      </Field>
      {!existing && administrator && <ScopeFields agentName={agent.name} />}
      <Alert>{error}</Alert>
      <div className="actions">
        <button className="btn btn--primary btn--sm" disabled={busy}>
          {busy ? 'Saving…' : 'Save'}
        </button>
        {fill && (
          <button
            type="button"
            className="btn btn--sm"
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
        <button
          type="button"
          className="btn btn--ghost btn--sm"
          onClick={onDone}
        >
          Cancel
        </button>
      </div>
    </form>
  )
}

export function LoginFields({
  existing,
  site,
}: {
  existing: VaultItem | undefined
  site: string | undefined
}) {
  return (
    <div className="row">
      <Field label="Site">
        <input
          name="site"
          required
          maxLength={255}
          placeholder="github.com"
          defaultValue={existing?.site ?? site ?? ''}
        />
      </Field>
      <Field label="Username">
        <input
          name="username"
          required
          maxLength={255}
          autoComplete="off"
          defaultValue={existing?.username ?? ''}
        />
      </Field>
    </div>
  )
}

function ScopeFields({ agentName }: { agentName: string }) {
  return (
    <div className="row">
      <Field label="Scope">
        <select name="scope" defaultValue="agent">
          <option value="agent">This agent only</option>
          <option value="installation">Installation</option>
        </select>
      </Field>
      <Field label="Installation access">
        <select name="access" defaultValue="all">
          <option value="all">All agents</option>
          <option value="this">Only {agentName}</option>
        </select>
      </Field>
    </div>
  )
}

function itemBody(
  data: FormData,
  kind: Kind,
  agent: string,
  existing: VaultItem | undefined,
) {
  const text = (name: string) => String(data.get(name) ?? '')
  const [secret, username] = [text('secret'), text('username')]
  if (!TYPABLE.test(secret) || !TYPABLE.test(username)) return null
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

export async function fillItem(
  agent: string,
  item: string,
  field: 'username' | 'secret',
): Promise<string | null> {
  const { response, error } = await api.POST(
    '/api/agents/{id}/vault/{item}/fill',
    {
      params: { path: { id: agent, item } },
      body: { field },
    },
  )
  if (response.ok) return null
  const reason = (error as { error?: string } | undefined)?.error
  return reason ?? new ApiError(response.status).message
}

export function agentVault(agent: string) {
  return ok(
    api.GET('/api/agents/{id}/vault', { params: { path: { id: agent } } }),
  )
}
