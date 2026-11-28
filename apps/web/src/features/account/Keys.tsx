import { useQuery, useQueryClient } from '@tanstack/react-query'
import { KeyRound, Plus } from 'lucide-react'
import { type FormEvent, useState } from 'react'
import { account, type ProviderKind } from '../../api/account'
import { ApiError, failure } from '../../api/client'
import type { components } from '../../api/schema'
import { useStanding } from '../../app/queries'
import {
  Alert,
  confirmAction,
  Field,
  Skeleton,
  Status,
  when,
} from '../../components/primitives'
import { PROVIDER_NAMES, tokens } from '../platform/models'

type Saved = components['schemas']['PersonalProvider']

const KEYS_KEY = ['account-providers']

/** Pay-as-you-go keys only; plans sign in through the installation. */
const PERSONAL: ProviderKind[] = [
  'openai',
  'anthropic',
  'openrouter',
  'gemini',
  'grok',
  'minimax',
  'minimax_cn',
]

/** Keys an agent you own runs on when it chooses that provider. */
export function Keys() {
  const standing = useStanding()
  const saved = useQuery({ queryKey: KEYS_KEY, queryFn: account.providers })
  const [editing, setEditing] = useState<ProviderKind | 'new' | null>(null)
  const [notice, setNotice] = useState('')
  const allowed = standing.data?.can_create.providers === true
  const keys = saved.data ?? []
  if (!standing.data || (!allowed && keys.length === 0)) return null
  return (
    <section className="profile__about" aria-labelledby="ai-keys">
      <div>
        <h3 id="ai-keys">Your AI keys</h3>
        <p className="faint small">
          An agent you own that chooses one of these providers in its model
          picker runs on your key, before any installation key. Keys are
          encrypted on the server and never shown again.
        </p>
      </div>
      {!allowed && (
        <p className="notice">
          An administrator turned off bringing your own keys. You can still
          forget the ones saved here.
        </p>
      )}
      {saved.isPending && <Skeleton lines={2} />}
      <Alert>{saved.isError ? failure(saved.error) : ''}</Alert>
      <Status>{notice}</Status>
      {keys.length > 0 && (
        <ul className="entries" aria-label="Your AI keys">
          {keys.map((key) => (
            <KeyRow
              key={key.provider}
              saved={key}
              allowed={allowed}
              onReplace={() => {
                setNotice('')
                setEditing(key.provider)
              }}
              onForgotten={setNotice}
            />
          ))}
        </ul>
      )}
      {allowed && (
        <KeyEditor
          editing={editing}
          saved={keys}
          onEdit={(next) => {
            setNotice('')
            setEditing(next)
          }}
          onSaved={(message) => {
            setEditing(null)
            setNotice(message)
          }}
        />
      )}
    </section>
  )
}

function KeyRow({
  saved,
  allowed,
  onReplace,
  onForgotten,
}: {
  saved: Saved
  allowed: boolean
  onReplace: () => void
  onForgotten: (message: string) => void
}) {
  const client = useQueryClient()
  const [error, setError] = useState('')
  const name = PROVIDER_NAMES[saved.provider]
  async function forget() {
    const yes = await confirmAction({
      title: `Forget your ${name} key?`,
      body: 'Your agents that choose this provider fall back to the installation’s key from their next turn, or stop if it has none.',
      action: 'Forget key',
    })
    if (!yes) return
    setError('')
    try {
      await account.forgetProvider(saved.provider)
      await client.invalidateQueries({ queryKey: KEYS_KEY })
      onForgotten(`Your ${name} key is gone.`)
    } catch (caught) {
      setError(failure(caught))
    }
  }
  return (
    <li className="entry">
      <span className="entry__icon" aria-hidden="true">
        <KeyRound />
      </span>
      <div className="entry__main">
        <span className="entry__name">{name}</span>
        <span className="entry__detail">
          Saved {when(saved.updated_at)}
          {saved.context_window
            ? ` · ${tokens(saved.context_window)} window`
            : ''}
        </span>
        <Alert>{error}</Alert>
      </div>
      <div className="actions">
        {allowed && (
          <button
            type="button"
            className="btn btn--ghost btn--sm"
            onClick={onReplace}
          >
            Replace
          </button>
        )}
        <button
          type="button"
          className="btn btn--ghost btn--sm"
          onClick={() => void forget()}
        >
          Forget
        </button>
      </div>
    </li>
  )
}

function KeyEditor({
  editing,
  saved,
  onEdit,
  onSaved,
}: {
  editing: ProviderKind | 'new' | null
  saved: Saved[]
  onEdit: (editing: ProviderKind | 'new' | null) => void
  onSaved: (message: string) => void
}) {
  if (editing === null)
    return (
      <div>
        <button
          type="button"
          className="btn btn--sm"
          onClick={() => onEdit('new')}
        >
          <Plus /> Add a key
        </button>
      </div>
    )
  const first = PERSONAL.find((p) => !saved.some((s) => s.provider === p))
  return (
    <KeyForm
      key={editing}
      provider={editing === 'new' ? (first ?? 'openai') : editing}
      fixed={editing !== 'new'}
      onCancel={() => onEdit(null)}
      onSaved={onSaved}
    />
  )
}

function KeyForm({
  provider,
  fixed,
  onCancel,
  onSaved,
}: {
  provider: ProviderKind
  fixed: boolean
  onCancel: () => void
  onSaved: (message: string) => void
}) {
  const client = useQueryClient()
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  async function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    const form = new FormData(event.currentTarget)
    const chosen = String(form.get('provider')) as ProviderKind
    const contextWindow = Number(form.get('window')) || undefined
    setBusy(true)
    setError('')
    try {
      await account.saveProvider(
        chosen,
        String(form.get('key')).trim(),
        contextWindow,
      )
      await client.invalidateQueries({ queryKey: KEYS_KEY })
      onSaved(
        `Your ${PROVIDER_NAMES[chosen]} key is saved. Your agents on ${PROVIDER_NAMES[chosen]} use it from their next turn.`,
      )
    } catch (caught) {
      setError(
        caught instanceof ApiError && caught.status === 403
          ? 'This installation does not let members bring their own keys.'
          : failure(caught),
      )
      setBusy(false)
    }
  }
  return (
    <form className="form you__form" onSubmit={(e) => void submit(e)}>
      <div className="profile__pair">
        <Field label="Provider">
          <select name="provider" defaultValue={provider} disabled={fixed}>
            {PERSONAL.map((id) => (
              <option key={id} value={id}>
                {PROVIDER_NAMES[id]}
              </option>
            ))}
          </select>
        </Field>
        <Field label="Context window" hint="Optional, in tokens">
          <input
            name="window"
            type="number"
            min={1000}
            step={1000}
            inputMode="numeric"
            placeholder="The model’s own"
          />
        </Field>
      </div>
      {/* A disabled select is left out of the form. */}
      {fixed && <input type="hidden" name="provider" value={provider} />}
      <Field label="API key">
        <input
          name="key"
          type="password"
          required
          autoComplete="off"
          spellCheck={false}
          autoFocus
        />
      </Field>
      <Alert>{error}</Alert>
      <div className="actions">
        <button className="btn btn--primary btn--sm" disabled={busy}>
          {busy ? 'Saving…' : fixed ? 'Replace key' : 'Save key'}
        </button>
        <button
          type="button"
          className="btn btn--ghost btn--sm"
          onClick={onCancel}
        >
          Cancel
        </button>
      </div>
    </form>
  )
}
