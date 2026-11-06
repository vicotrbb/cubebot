import { useQuery, useQueryClient } from '@tanstack/react-query'
import { createFileRoute, useNavigate } from '@tanstack/react-router'
import { type SubmitEvent, useState } from 'react'
import { ApiError, api, failure, ok } from '../api/client'
import { Cube } from '../components/Cube'
import { Alert, Field } from '../components/primitives'
import '../features/shell/sign-in.css'

export const Route = createFileRoute('/sign-in')({
  head: () => ({ meta: [{ title: 'Sign in · Cubebot' }] }),
  component: SignIn,
})

function SignIn() {
  const setup = useQuery({
    queryKey: ['setup'],
    queryFn: () => ok(api.GET('/api/identity/setup')),
  })
  const open = setup.data?.open === true
  return (
    <main className="sign-in">
      <div className="sign-in__card">
        <div className="sign-in__brand">
          <Cube size={48} follow />
          <span className="sign-in__name">Cubebot</span>
        </div>
        {setup.isPending ? null : open ? (
          <>
            <h1>Welcome.</h1>
            <p className="muted">
              This installation has no accounts yet. Create the administrator.
            </p>
            <Credentials setup />
          </>
        ) : (
          <>
            <h1>Welcome back.</h1>
            <p className="muted">Sign in to your installation.</p>
            <Credentials setup={false} />
            <SingleSignOn />
            <p className="small faint">
              Accounts are managed by your installation administrator.
            </p>
          </>
        )}
      </div>
    </main>
  )
}

/** Offered only while the installation has single sign-on on. */
function SingleSignOn() {
  const status = useQuery({
    queryKey: ['single-sign-on-status'],
    queryFn: () => ok(api.GET('/api/identity/oidc')),
  })
  const [error, setError] = useState('')
  const [busy, setBusy] = useState(false)
  if (!status.data?.enabled) return null
  async function begin() {
    setBusy(true)
    setError('')
    try {
      const started = await ok(api.POST('/api/identity/oidc/start'))
      window.location.assign(started.authorization_url)
    } catch (caught) {
      setError(
        caught instanceof ApiError && caught.status === 503
          ? 'Your identity provider is not answering. Try again shortly, or sign in with a password.'
          : failure(caught),
      )
      setBusy(false)
    }
  }
  return (
    <div className="sign-in__sso">
      <span className="sign-in__or">or</span>
      <Alert>{error}</Alert>
      <button
        type="button"
        className="btn"
        disabled={busy}
        onClick={() => void begin()}
      >
        {busy
          ? 'Opening your identity provider…'
          : 'Continue with single sign-on'}
      </button>
    </div>
  )
}

const LABELS = {
  true: ['Create administrator', 'Creating…'],
  false: ['Sign in', 'Signing in…'],
}

function Credentials({ setup }: { setup: boolean }) {
  const navigate = useNavigate()
  const client = useQueryClient()
  const [error, setError] = useState('')
  const [busy, setBusy] = useState(false)
  async function submit(event: SubmitEvent<HTMLFormElement>) {
    event.preventDefault()
    const data = new FormData(event.currentTarget)
    const body = {
      login: String(data.get('login')),
      password: String(data.get('password')),
    }
    if (setup && body.password !== data.get('confirm')) {
      setError('The passwords do not match.')
      return
    }
    setBusy(true)
    setError('')
    try {
      await ok(
        setup
          ? api.POST('/api/identity/setup', { body })
          : api.POST('/api/identity/login', { body }),
      )
      // Nothing cached belongs to this account, and navigating on a cached null session would send the browser straight back to sign-in.
      client.clear()
      await navigate({ to: '/' })
    } catch (error) {
      setError(failure(error))
      // Another administrator may have claimed the installation meanwhile.
      if (setup) await client.invalidateQueries({ queryKey: ['setup'] })
    } finally {
      setBusy(false)
    }
  }
  return (
    <form className="form" onSubmit={submit}>
      <Field label="Login">
        <input
          name="login"
          autoComplete="username"
          required
          maxLength={64}
          autoCapitalize="none"
          autoFocus
        />
      </Field>
      <Field label="Password">
        <input
          name="password"
          type="password"
          autoComplete={setup ? 'new-password' : 'current-password'}
          minLength={setup ? 12 : undefined}
          required
        />
      </Field>
      {setup && (
        <Field label="Confirm password">
          <input
            name="confirm"
            type="password"
            autoComplete="new-password"
            required
          />
        </Field>
      )}
      <Alert>{error}</Alert>
      <button className="btn btn--primary" disabled={busy}>
        {LABELS[`${setup}`][busy ? 1 : 0]}
      </button>
    </form>
  )
}
