import { useState } from 'react'
import { request, failure, provider, type Agent } from '../api/client'

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
            await request('/identity/login', {
              login: data.get('login'),
              password: data.get('password'),
            })
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
            name: data.get('name'),
            instructions: data.get('instructions'),
          }
          try {
            if (existing)
              await request(`/agents/${existing.id}/definition`, {
                ...input,
                expected_revision: existing.revision,
                idle_policy: existing.idle_policy,
              })
            else await request('/agents', input)
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
  const [model, setModel] = useState('')
  const [error, setError] = useState('')
  const [busy, setBusy] = useState(false)
  const [saved, setSaved] = useState(false)
  const [requestId, setRequestId] = useState(() => crypto.randomUUID())
  async function load() {
    setBusy(true)
    setError('')
    try {
      const current = provider(await request('/provider'))
      setRevision(current?.revision ?? 0)
      setModel(current?.model ?? '')
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
        One OpenAI connection and model for eligible agents. Only administrators
        can change this setting.
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
              const current = provider(
                await request('/provider', {
                  request_id: requestId,
                  expected_revision: revision,
                  model,
                  api_key: input.get('api_key'),
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
