import { useState } from 'react'
import { api, failure, ok, type Agent } from '../api/client'

export function SignIn({ onSignedIn }: { onSignedIn: () => Promise<void> }) {
  const [error, setError] = useState('')
  const [busy, setBusy] = useState(false)
  return (
    <main className="sign-in">
      <a className="wordmark" href="/">
        cubebot
      </a>
      <h1>Welcome back.</h1>
      <p>Sign in to your installation.</p>
      <form
        onSubmit={async (event) => {
          event.preventDefault()
          setBusy(true)
          setError('')
          const data = new FormData(event.currentTarget)
          try {
            await ok(
              api.POST('/api/identity/login', {
                body: {
                  login: String(data.get('login')),
                  password: String(data.get('password')),
                },
              }),
            )
            await onSignedIn()
          } catch (error) {
            setError(failure(error))
          } finally {
            setBusy(false)
          }
        }}
      >
        <label>
          Login
          <input
            name="login"
            autoComplete="username"
            required
            maxLength={64}
            autoCapitalize="none"
          />
        </label>
        <label>
          Password
          <input
            name="password"
            type="password"
            autoComplete="current-password"
            required
          />
        </label>
        {error && <p role="alert">{error}</p>}
        <button disabled={busy}>{busy ? 'Signing in…' : 'Sign in'}</button>
      </form>
      <p className="muted">
        Accounts are managed by your installation administrator.
      </p>
    </main>
  )
}

export function AgentForm({
  existing,
  onSaved,
  onCancel,
}: {
  existing: Agent | null
  onSaved: () => Promise<void>
  onCancel: () => void
}) {
  const [error, setError] = useState('')
  const [busy, setBusy] = useState(false)
  const [requestId] = useState(() => crypto.randomUUID())
  return (
    <section className="settings">
      <h2>{existing ? 'Edit agent' : 'Create an agent'}</h2>
      <form
        onSubmit={async (event) => {
          event.preventDefault()
          setBusy(true)
          setError('')
          const data = new FormData(event.currentTarget)
          const input = {
            request_id: requestId,
            name: String(data.get('name')),
            instructions: String(data.get('instructions')),
          }
          try {
            if (existing)
              await ok(
                api.POST('/api/agents/{id}/definition', {
                  params: { path: { id: existing.id } },
                  body: {
                    ...input,
                    expected_revision: existing.revision,
                    idle_policy: existing.idle_policy,
                    idle_delay_seconds: Number(data.get('idle_minutes')) * 60,
                    dreaming_model:
                      String(data.get('dreaming_model')).trim() || null,
                    task_lanes: Number(data.get('task_lanes')),
                  },
                }),
              )
            else await ok(api.POST('/api/agents', { body: input }))
            await onSaved()
          } catch (error) {
            setError(failure(error))
          } finally {
            setBusy(false)
          }
        }}
      >
        <label>
          Name
          <input
            name="name"
            defaultValue={existing?.name}
            required
            maxLength={255}
          />
        </label>
        <label>
          Instructions
          <textarea
            name="instructions"
            defaultValue={existing?.instructions}
            rows={7}
            maxLength={65536}
          />
        </label>
        {existing && (
          <>
            <label>
              Idle minutes before dreaming
              <input
                name="idle_minutes"
                type="number"
                required
                min={1}
                max={1440}
                defaultValue={Math.round(existing.idle_delay_seconds / 60)}
              />
            </label>
            <label>
              Delegated tasks at once
              <input
                name="task_lanes"
                type="number"
                required
                min={1}
                max={4}
                defaultValue={existing.task_lanes}
              />
            </label>
            <label>
              Dreaming model
              <input
                name="dreaming_model"
                maxLength={200}
                placeholder="Same as the provider model"
                defaultValue={existing.dreaming_model ?? ''}
              />
            </label>
          </>
        )}
        {error && <p role="alert">{error}</p>}
        <div className="actions">
          <button disabled={busy}>{busy ? 'Saving…' : 'Save agent'}</button>
          <button type="button" className="secondary" onClick={onCancel}>
            Cancel
          </button>
        </div>
      </form>
    </section>
  )
}

export function ProviderForm({ onClose }: { onClose: () => void }) {
  const [revision, setRevision] = useState<number | null>(null)
  const [kind, setKind] = useState<'openai' | 'anthropic'>('openai')
  const [model, setModel] = useState('')
  const [window, setWindow] = useState('')
  const [error, setError] = useState('')
  const [busy, setBusy] = useState(false)
  const [saved, setSaved] = useState(false)
  const [requestId, setRequestId] = useState(() => crypto.randomUUID())
  async function load() {
    setBusy(true)
    setError('')
    try {
      const current = await ok(api.GET('/api/provider'))
      setRevision(current?.revision ?? 0)
      setKind(current?.provider ?? 'openai')
      setModel(current?.model ?? '')
      setWindow(current?.context_window?.toString() ?? '')
    } catch (error) {
      setError(failure(error))
    } finally {
      setBusy(false)
    }
  }
  return (
    <section className="settings">
      <h2>AI provider</h2>
      <p>
        One OpenAI or Anthropic connection and model for eligible agents. Only
        administrators can change this setting.
      </p>
      {revision === null ? (
        <button disabled={busy} onClick={load}>
          {busy ? 'Loading…' : 'Load provider settings'}
        </button>
      ) : (
        <form
          onSubmit={async (event) => {
            event.preventDefault()
            setBusy(true)
            setError('')
            setSaved(false)
            const form = event.currentTarget
            const input = new FormData(form)
            try {
              const current = await ok(
                api.POST('/api/provider', {
                  body: {
                    request_id: requestId,
                    expected_revision: revision,
                    provider: kind,
                    model,
                    context_window: window === '' ? null : Number(window),
                    api_key: String(input.get('api_key')),
                  },
                }),
              )
              setRevision(current?.revision ?? 0)
              setSaved(true)
              setRequestId(crypto.randomUUID())
              form.reset()
            } catch (error) {
              setError(failure(error))
            } finally {
              setBusy(false)
            }
          }}
        >
          <label>
            Provider
            <select
              value={kind}
              onChange={(event) =>
                setKind(event.target.value as 'openai' | 'anthropic')
              }
            >
              <option value="openai">OpenAI</option>
              <option value="anthropic">Anthropic</option>
            </select>
          </label>
          <label>
            Model
            <input
              value={model}
              onChange={(event) => setModel(event.target.value)}
              required
              maxLength={128}
              placeholder="Enter an available model ID"
            />
          </label>
          <label>
            Context window (tokens)
            <input
              value={window}
              onChange={(event) => setWindow(event.target.value)}
              type="number"
              min={1024}
              max={100000000}
              required={kind === 'openai'}
              placeholder={
                kind === 'anthropic'
                  ? 'Reported by Anthropic when left empty'
                  : 'The model window, for example 128000'
              }
            />
          </label>
          <p className="muted">
            Conversations are summarized when the provider reports 80% of this
            window in use.
          </p>
          <label>
            API key
            <input
              name="api_key"
              type="password"
              autoComplete="off"
              required
              maxLength={4096}
            />
          </label>
          <p className="muted">
            The key is encrypted on the server and is never returned here. Enter
            a key to save or replace the connection.
          </p>
          <button disabled={busy}>{busy ? 'Saving…' : 'Save provider'}</button>
        </form>
      )}
      {error && <p role="alert">{error}</p>}
      {saved && <p role="status">Provider settings saved.</p>}
      <button className="secondary" onClick={onClose}>
        Back to agents
      </button>
    </section>
  )
}
