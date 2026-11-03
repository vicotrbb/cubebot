import { useQuery, useQueryClient } from '@tanstack/react-query'
import { KeyRound, RefreshCw } from 'lucide-react'
import { useState } from 'react'
import { api, failure, ok } from '../../api/client'
import type { components } from '../../api/schema'
import { Alert, Field, Skeleton, Status } from '../../components/primitives'
import { COMMON_WINDOWS, suggestedWindow, tokens } from './models'
import './platform.css'

type Provider = components['schemas']['ProviderKind']
type Current = components['schemas']['ProviderSummary']
type Choice = components['schemas']['ModelChoice']

const PROVIDERS: { id: Provider; name: string; note: string }[] = [
  {
    id: 'openai',
    name: 'OpenAI',
    note: 'Responses API. You set the context window.',
  },
  {
    id: 'anthropic',
    name: 'Anthropic',
    note: 'Messages API. Each model reports its own window.',
  },
]
const NAMES: Record<Provider, string> = {
  openai: 'OpenAI',
  anthropic: 'Anthropic',
}

/** One installation-managed provider connection (docs/adr/identity.md#installation-managed-provider-connections and docs/adr/agents.md#initial-openai-anthropic-and-compatible-endpoint-support):
 *  the provider, then its key, then a model the key lists, then that model's window. */
export function Providers() {
  const client = useQueryClient()
  const current = useQuery({
    queryKey: ['provider'],
    queryFn: () => ok(api.GET('/api/provider')),
  })
  // Here, not in the form, which starts over on the revision a save makes.
  const [notice, setNotice] = useState('')
  return (
    <div className="provider-page">
      <p className="platform__lead">
        Every eligible agent thinks with this connection. The key is encrypted
        on the server and never shown again.
      </p>
      {current.isPending && <Skeleton lines={4} width="62ch" />}
      {current.data && <Connection current={current.data} />}
      <Status>{notice}</Status>
      {current.isSuccess && (
        <ProviderForm
          key={current.data?.revision ?? 0}
          current={current.data ?? null}
          onEdit={() => setNotice('')}
          onSaved={async () => {
            setNotice('Connection saved. Agents use it from their next turn.')
            await client.invalidateQueries({ queryKey: ['provider'] })
          }}
        />
      )}
    </div>
  )
}

function Connection({ current }: { current: Current }) {
  return (
    <section className="provider-current" aria-label="Current connection">
      <span className="dot dot--idle" aria-hidden="true" />
      <div>
        <p className="provider-current__title">
          {NAMES[current.provider]} · <code>{current.model}</code>
        </p>
        <p className="small faint">
          {current.context_window
            ? `${tokens(current.context_window)} token window`
            : 'Window reported by the provider'}{' '}
          · revision {current.revision}
        </p>
      </div>
    </section>
  )
}

/** Lists the models a key can use: the saved key's on their own whenever it applies, a
 *  pasted key's once asked. */
function useModels(provider: Provider, key: string, saved: boolean) {
  // The pasted key a person asked to list with.
  const [asked, setAsked] = useState<string | null>(null)
  // '' lists with the saved key; null lists nothing yet.
  const listing = saved && !key ? '' : asked
  const query = useQuery({
    queryKey: ['provider-models', provider, listing],
    enabled: listing !== null,
    retry: false,
    // A pasted key lives in this cache only while the form shows its models.
    gcTime: 0,
    queryFn: async () => {
      const { data, response } = await api
        .POST('/api/provider/models', {
          body: { provider, api_key: listing || null },
        })
        .catch(() => ({ data: undefined, response: null }))
      if (!data)
        throw new Error(listProblem(response?.status, provider), {
          cause: unreachable(response?.status),
        })
      return data
    },
  })
  return {
    models: query.data ?? null,
    problem: query.error?.message ?? '',
    // Unreachable, not refused: a person may type the model instead.
    unreachable: query.error?.cause === true,
    loading: query.isFetching,
    load: () => {
      if (listing === (key || (saved ? '' : null))) void query.refetch()
      else setAsked(key)
    },
    reset: () => setAsked(null),
  }
}

/** What the form starts from for `provider`: the saved model and window when it is the
 *  saved provider, nothing otherwise. */
function draftFor(current: Current | null, provider: Provider) {
  if (current?.provider !== provider) return { model: '', window: '' }
  return {
    model: current.model,
    window: current.context_window ? String(current.context_window) : '',
  }
}

/** Saves the connection; one request id per attempt, so a retried save is not a second one. */
function useSaveConnection(
  current: Current | null,
  onSaved: () => Promise<unknown>,
) {
  const [error, setError] = useState('')
  const [busy, setBusy] = useState(false)
  const [requestId, setRequestId] = useState(() => crypto.randomUUID())
  async function save(body: {
    provider: Provider
    model: string
    window: string
    key: string
  }) {
    setBusy(true)
    setError('')
    try {
      await ok(
        api.POST('/api/provider', {
          body: {
            request_id: requestId,
            expected_revision: current?.revision ?? 0,
            provider: body.provider,
            model: body.model,
            context_window: body.window === '' ? null : Number(body.window),
            api_key: body.key || null,
          },
        }),
      )
      setRequestId(crypto.randomUUID())
      await onSaved()
    } catch (error) {
      setError(failure(error))
    } finally {
      setBusy(false)
    }
  }
  return { save, error, busy }
}

const unreachable = (status: number | undefined) =>
  status === 502 || status === undefined

function listProblem(status: number | undefined, provider: Provider) {
  if (status === 422)
    return `${NAMES[provider]} rejected this key. Check it and try again.`
  if (unreachable(status))
    return `${NAMES[provider]} could not be reached. Type the model ID instead, or retry.`
  if (status === 400) return 'That key has characters a key cannot hold.'
  return 'The models could not be listed. Retry shortly.'
}

function ProviderForm({
  current,
  onEdit,
  onSaved,
}: {
  current: Current | null
  onEdit: () => void
  onSaved: () => Promise<unknown>
}) {
  const [provider, setProvider] = useState<Provider>(
    current?.provider ?? 'openai',
  )
  const [key, setKey] = useState('')
  const [draft, setDraft] = useState(() => draftFor(current, provider))
  // A saved key lists and saves without being pasted again, for the same provider only.
  const saved = current?.provider === provider
  const list = useModels(provider, key, saved)
  const connection = useSaveConnection(current, onSaved)
  const choose = (id: string) => {
    const reported = list.models?.find((choice) => choice.id === id)
    const suggestion = suggestedWindow(id, reported?.context_window)
    setDraft({ model: id, window: suggestion ? String(suggestion.tokens) : '' })
    onEdit()
  }
  const ready = Boolean(draft.model) && (Boolean(key) || saved)
  return (
    <form
      className="form provider-form"
      onSubmit={(event) => {
        event.preventDefault()
        void connection.save({ provider, key, ...draft })
      }}
    >
      <h2>{current ? 'Change the connection' : 'Connect a provider'}</h2>
      <ProviderChoice
        value={provider}
        onPick={(next) => {
          setProvider(next)
          setKey('')
          list.reset()
          setDraft(draftFor(current, next))
          onEdit()
        }}
      />
      <KeyField
        provider={provider}
        value={key}
        saved={saved}
        loading={list.loading}
        onChange={(value) => {
          setKey(value)
          list.reset()
        }}
        onLoad={() => void list.load()}
      />
      {list.problem && (
        <p role="alert" className="small provider-problem">
          {list.problem}
        </p>
      )}
      <ModelField
        models={list.models}
        typed={list.unreachable}
        value={draft.model}
        onPick={choose}
      />
      <WindowField
        provider={provider}
        model={draft.model}
        reported={
          list.models?.find((choice) => choice.id === draft.model)
            ?.context_window
        }
        value={draft.window}
        onChange={(window) => setDraft({ ...draft, window })}
      />
      <Alert>{connection.error}</Alert>
      <div className="actions">
        <button
          className="btn btn--primary"
          disabled={connection.busy || !ready}
        >
          {connection.busy ? 'Saving…' : 'Save connection'}
        </button>
      </div>
    </form>
  )
}

function ProviderChoice({
  value,
  onPick,
}: {
  value: Provider
  onPick: (provider: Provider) => void
}) {
  return (
    <fieldset className="provider-choice">
      <legend>Provider</legend>
      {PROVIDERS.map((option) => (
        <label key={option.id} className="provider-choice__option">
          <input
            type="radio"
            name="provider"
            value={option.id}
            checked={value === option.id}
            onChange={() => onPick(option.id)}
          />
          <span className="provider-choice__name">{option.name}</span>
          <span className="provider-choice__note">{option.note}</span>
        </label>
      ))}
    </fieldset>
  )
}

function KeyField({
  provider,
  value,
  saved,
  loading,
  onChange,
  onLoad,
}: {
  provider: Provider
  value: string
  saved: boolean
  loading: boolean
  onChange: (value: string) => void
  onLoad: () => void
}) {
  return (
    <div className="provider-key">
      <Field
        label="API key"
        hint={saved ? 'Empty keeps the saved key' : 'Never shown again'}
      >
        <input
          type="password"
          autoComplete="off"
          spellCheck={false}
          maxLength={4096}
          value={value}
          placeholder={
            saved
              ? 'Saved key in use; paste a new one to replace it'
              : `Your ${NAMES[provider]} API key`
          }
          onChange={(event) => onChange(event.target.value.trim())}
          onBlur={() => value && onLoad()}
          onKeyDown={(event) => {
            if (event.key !== 'Enter') return
            event.preventDefault()
            onLoad()
          }}
        />
      </Field>
      <button
        type="button"
        className="btn"
        disabled={loading || (!value && !saved)}
        onClick={onLoad}
      >
        {loading ? <RefreshCw className="spin" /> : <KeyRound />}
        {loading ? 'Checking…' : 'List models'}
      </button>
    </div>
  )
}

function ModelField({
  models,
  typed,
  value,
  onPick,
}: {
  models: Choice[] | null
  /** The provider could not be reached, so the ID is typed; an outage never blocks setup. */
  typed: boolean
  value: string
  onPick: (id: string) => void
}) {
  const locked = !models
  if (typed)
    return (
      <Field label="Model">
        <input
          required
          maxLength={128}
          value={value}
          placeholder="A model ID the key can use"
          onChange={(event) => onPick(event.target.value.trim())}
        />
      </Field>
    )
  const ids = models?.map((choice) => choice.id) ?? []
  const options = value && !ids.includes(value) ? [value, ...ids] : ids
  return (
    <Field
      label="Model"
      hint={models ? `${models.length} available, newest first` : undefined}
    >
      <select
        required
        disabled={locked}
        value={value}
        onChange={(event) => onPick(event.target.value)}
      >
        <option value="" disabled>
          {locked ? 'Enter the key to list its models' : 'Choose a model'}
        </option>
        {options.map((id) => (
          <option key={id} value={id}>
            {id}
          </option>
        ))}
      </select>
    </Field>
  )
}

function WindowField({
  provider,
  model,
  reported,
  value,
  onChange,
}: {
  provider: Provider
  model: string
  reported: number | null | undefined
  value: string
  onChange: (value: string) => void
}) {
  const suggestion = model ? suggestedWindow(model, reported) : null
  const sizes = [
    ...new Set([...(suggestion ? [suggestion.tokens] : []), ...COMMON_WINDOWS]),
  ]
  return (
    <div className="provider-group">
      <Field label="Context window" hint={windowHint(provider, suggestion)}>
        <input
          type="number"
          inputMode="numeric"
          min={1024}
          max={100000000}
          required={provider === 'openai'}
          value={value}
          placeholder={
            provider === 'anthropic' ? 'Reported by Anthropic' : 'Tokens'
          }
          onChange={(event) => onChange(event.target.value)}
        />
      </Field>
      <span
        className="provider-sizes"
        role="group"
        aria-label="Suggested sizes"
      >
        {sizes.map((size) => (
          <button
            key={size}
            type="button"
            className="chip provider-size"
            aria-pressed={value === String(size)}
            onClick={() => onChange(String(size))}
          >
            {tokens(size)}
            {size === suggestion?.tokens && (
              <span className="provider-size__tag">
                {suggestion.source === 'reported' ? 'reported' : 'suggested'}
              </span>
            )}
          </button>
        ))}
      </span>
      <span className="small faint">
        Conversations are summarized at 80% of the window.
      </span>
    </div>
  )
}

function windowHint(
  provider: Provider,
  suggestion: ReturnType<typeof suggestedWindow>,
) {
  if (suggestion?.source === 'reported') return 'Reported by the provider'
  if (suggestion) return "Typical for this model's family"
  return provider === 'anthropic' ? 'Empty uses the model’s report' : undefined
}
