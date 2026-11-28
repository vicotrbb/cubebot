import { useQuery, useQueryClient } from '@tanstack/react-query'
import { createFileRoute, useNavigate } from '@tanstack/react-router'
import { type SubmitEvent, useState } from 'react'
import { ApiError, api, failure, ok } from '../api/client'
import { Cube } from '../components/Cube'
import { Alert, Field } from '../components/primitives'
import { CodeField, codeOf } from '../features/account/CodeField'
import { signInRefusal } from '../features/account/model'
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

type Login = { login: string; password: string }

/** Signs in; answers whether the account still needs its two-factor code. */
async function signIn(body: Login & { code?: string }) {
  const { error, response } = await api.POST('/api/identity/login', { body })
  if (response.ok) return 'done'
  const refusal = signInRefusal(response.status, error)
  if (refusal === 'code') return 'code'
  if (refusal === 'wrong')
    throw new Error(
      body.code
        ? 'That code did not work. Enter the one showing now, or a recovery code you have not used.'
        : 'That login and password do not match an account.',
    )
  throw new ApiError(response.status)
}

/** Nothing cached belongs to this account, and navigating on a cached null session would send the browser straight back to sign-in. */
function useEnter() {
  const navigate = useNavigate()
  const client = useQueryClient()
  return async () => {
    client.clear()
    await navigate({ to: '/' })
  }
}

function Credentials({ setup }: { setup: boolean }) {
  const [waiting, setWaiting] = useState<Login | null>(null)
  if (waiting)
    return <SecondStep login={waiting} onBack={() => setWaiting(null)} />
  return <Password setup={setup} onCode={setWaiting} />
}

function Password({
  setup,
  onCode,
}: {
  setup: boolean
  onCode: (login: Login) => void
}) {
  const enter = useEnter()
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
      if (setup) await ok(api.POST('/api/identity/setup', { body }))
      else if ((await signIn(body)) === 'code') return onCode(body)
      await enter()
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

/** The account has two-factor on: the same login and password again, with a code. */
function SecondStep({ login, onBack }: { login: Login; onBack: () => void }) {
  const enter = useEnter()
  const [error, setError] = useState('')
  const [busy, setBusy] = useState(false)
  async function submit(event: SubmitEvent<HTMLFormElement>) {
    event.preventDefault()
    const code = codeOf(new FormData(event.currentTarget))
    setBusy(true)
    setError('')
    try {
      if ((await signIn({ ...login, code })) === 'done') await enter()
      else setError('Enter the code from your authenticator app.')
    } catch (error) {
      setError(failure(error))
    } finally {
      setBusy(false)
    }
  }
  return (
    <form className="form" onSubmit={submit}>
      <p className="small">
        <strong>{login.login}</strong> has two-factor sign-in on. Enter the
        six-digit code from your authenticator app.
      </p>
      <CodeField autoFocus />
      <Alert>{error}</Alert>
      <button className="btn btn--primary" disabled={busy}>
        {busy ? 'Checking…' : 'Verify and sign in'}
      </button>
      <button type="button" className="link-btn small" onClick={onBack}>
        Sign in as someone else
      </button>
    </form>
  )
}
