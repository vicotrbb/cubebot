import { createFileRoute, Link } from '@tanstack/react-router'
import { type SubmitEvent, useState } from 'react'
import { account } from '../api/account'
import { ApiError, failure } from '../api/client'
import { Cube } from '../components/Cube'
import { Alert, Field, Status } from '../components/primitives'
import { PASSWORD_MIN, passwordProblem } from '../features/account/model'
import '../features/shell/sign-in.css'

export const Route = createFileRoute('/reset-password')({
  head: () => ({ meta: [{ title: 'Set a new password · Cubebot' }] }),
  validateSearch: (search: Record<string, unknown>): { token?: string } =>
    typeof search.token === 'string' && /^[0-9a-f]{64}$/i.test(search.token)
      ? { token: search.token }
      : {},
  component: ResetPassword,
})

const EXPIRED =
  'This link was already used or has expired. Ask your administrator for a new one.'

function ResetPassword() {
  const { token } = Route.useSearch()
  return (
    <main className="sign-in">
      <div className="sign-in__card">
        <div className="sign-in__brand">
          <Cube size={48} follow />
          <span className="sign-in__name">Cubebot</span>
        </div>
        <h1>Set a new password.</h1>
        {token ? (
          <>
            <p className="muted">
              It replaces your old one and signs you out everywhere else.
            </p>
            <NewPassword token={token} />
          </>
        ) : (
          <>
            <Alert>
              This link is incomplete. Open the whole link your administrator
              sent, or ask them for a new one.
            </Alert>
            <Link to="/sign-in" className="btn">
              Go to sign in
            </Link>
          </>
        )}
      </div>
    </main>
  )
}

function NewPassword({ token }: { token: string }) {
  const [error, setError] = useState('')
  const [busy, setBusy] = useState(false)
  const [done, setDone] = useState(false)
  async function submit(event: SubmitEvent<HTMLFormElement>) {
    event.preventDefault()
    const data = new FormData(event.currentTarget)
    const password = String(data.get('password'))
    const problem = passwordProblem(password, String(data.get('confirm')))
    if (problem) return setError(problem)
    setBusy(true)
    setError('')
    try {
      await account.redeemReset(token, password)
      setDone(true)
    } catch (caught) {
      setError(
        caught instanceof ApiError && caught.status === 401
          ? EXPIRED
          : failure(caught),
      )
      setBusy(false)
    }
  }
  if (done)
    return (
      <>
        <Status>Your new password is set. Sign in with it.</Status>
        <Link to="/sign-in" className="btn btn--primary">
          Sign in
        </Link>
      </>
    )
  return (
    <form className="form" onSubmit={submit}>
      <Field label="New password" hint={`At least ${PASSWORD_MIN} characters`}>
        <input
          name="password"
          type="password"
          autoComplete="new-password"
          minLength={PASSWORD_MIN}
          required
          autoFocus
        />
      </Field>
      <Field label="Confirm new password">
        <input
          name="confirm"
          type="password"
          autoComplete="new-password"
          required
        />
      </Field>
      <Alert>{error}</Alert>
      <button className="btn btn--primary" disabled={busy}>
        {busy ? 'Saving…' : 'Set password'}
      </button>
    </form>
  )
}
