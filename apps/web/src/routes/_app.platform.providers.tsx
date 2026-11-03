import { useQuery, useQueryClient } from '@tanstack/react-query'
import { createFileRoute } from '@tanstack/react-router'
import { useState } from 'react'
import { api, failure, ok } from '../api/client'
import { Alert, Field, Skeleton, Status } from '../components/primitives'
import '../features/platform/platform.css'

export const Route = createFileRoute('/_app/platform/providers')({
  component: Providers,
})

type Provider = 'openai' | 'anthropic'
type Current = {
  provider: Provider
  model: string
  context_window?: number | null
  revision: number
}

/** One installation-managed provider connection (docs/adr/identity.md#installation-managed-provider-connections and docs/adr/agents.md#initial-openai-anthropic-and-compatible-endpoint-support). */
function Providers() {
  const client = useQueryClient()
  const current = useQuery({
    queryKey: ['provider'],
    queryFn: () => ok(api.GET('/api/provider')),
  })
  return (
    <div className="platform">
      <div>
        <p className="platform__lead">
          One OpenAI or Anthropic connection and model for eligible agents
          (installation-wide connection access: enabled connections reach every
          agent). The key is encrypted on the server and never shown again.
        </p>
        {current.isPending && <Skeleton lines={3} />}
        {current.data && (
          <dl className="kv">
            <dt>Provider</dt>
            <dd>{current.data.provider}</dd>
            <dt>Model</dt>
            <dd>{current.data.model}</dd>
            <dt>Context window</dt>
            <dd>{current.data.context_window ?? 'reported by the provider'}</dd>
            <dt>Revision</dt>
            <dd>{current.data.revision}</dd>
          </dl>
        )}
        {current.isSuccess && !current.data && (
          <p className="muted">No provider configured yet.</p>
        )}
      </div>
      <aside className="platform__editor">
        <h2>
          {current.data ? 'Replace the connection' : 'Configure a provider'}
        </h2>
        {!current.isPending && (
          <ProviderForm
            current={current.data ?? null}
            onSaved={() => client.invalidateQueries({ queryKey: ['provider'] })}
          />
        )}
      </aside>
    </div>
  )
}

function ProviderForm({
  current,
  onSaved,
}: {
  current: Current | null
  onSaved: () => Promise<unknown>
}) {
  const [provider, setProvider] = useState<Provider>(
    current?.provider ?? 'openai',
  )
  const [error, setError] = useState('')
  const [saved, setSaved] = useState(false)
  const [busy, setBusy] = useState(false)
  const [requestId, setRequestId] = useState(() => crypto.randomUUID())
  async function save(form: HTMLFormElement) {
    setBusy(true)
    setError('')
    setSaved(false)
    const data = new FormData(form)
    const window = String(data.get('context_window'))
    try {
      await ok(
        api.POST('/api/provider', {
          body: {
            request_id: requestId,
            expected_revision: current?.revision ?? 0,
            provider,
            model: String(data.get('model')),
            context_window: window === '' ? null : Number(window),
            api_key: String(data.get('api_key')),
          },
        }),
      )
      await onSaved()
      setSaved(true)
      setRequestId(crypto.randomUUID())
      form.reset()
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
      <Field label="Provider">
        <select
          value={provider}
          onChange={(event) => setProvider(event.target.value as Provider)}
        >
          <option value="openai">OpenAI</option>
          <option value="anthropic">Anthropic</option>
        </select>
      </Field>
      <Field label="Model">
        <input
          name="model"
          required
          maxLength={128}
          defaultValue={current?.model ?? ''}
          placeholder="A model ID the key can use"
        />
      </Field>
      <Field
        label="Context window (tokens)"
        hint={
          provider === 'anthropic'
            ? 'Reported by Anthropic when empty'
            : 'For example 128000'
        }
      >
        <input
          name="context_window"
          type="number"
          min={1024}
          max={100000000}
          required={provider === 'openai'}
          defaultValue={current?.context_window ?? ''}
        />
      </Field>
      <Field label="API key">
        <input
          name="api_key"
          type="password"
          autoComplete="off"
          required
          maxLength={4096}
        />
      </Field>
      <p className="small faint">
        Conversations are summarized when the provider reports 80% of the window
        in use.
      </p>
      <Alert>{error}</Alert>
      <Status>{saved ? 'Provider settings saved.' : ''}</Status>
      <div className="actions">
        <button className="btn btn--primary btn--sm" disabled={busy}>
          {busy ? 'Saving…' : 'Save provider'}
        </button>
      </div>
    </form>
  )
}
