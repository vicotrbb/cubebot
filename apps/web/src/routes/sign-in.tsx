import { useQueryClient } from '@tanstack/react-query'
import { createFileRoute, useNavigate } from '@tanstack/react-router'
import { useState } from 'react'
import { api, failure, ok } from '../api/client'
import { keys } from '../app/queries'
import { Cube } from '../components/Cube'
import { Alert, Field } from '../components/primitives'
import '../features/shell/sign-in.css'

export const Route = createFileRoute('/sign-in')({ component: SignIn })

function SignIn() {
  const navigate = useNavigate()
  const client = useQueryClient()
  const [error, setError] = useState('')
  const [busy, setBusy] = useState(false)
  return (
    <main className="sign-in">
      <div className="sign-in__card">
        <div className="sign-in__brand">
          <Cube size={64} follow />
          <span className="sign-in__name">Cubebot</span>
        </div>
        <h1>Welcome back.</h1>
        <p className="muted">Sign in to your installation.</p>
        <form
          className="form"
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
              await client.invalidateQueries({ queryKey: keys.session })
              await navigate({ to: '/' })
            } catch (error) {
              setError(failure(error))
            } finally {
              setBusy(false)
            }
          }}
        >
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
              autoComplete="current-password"
              required
            />
          </Field>
          <Alert>{error}</Alert>
          <button className="btn btn--primary" disabled={busy}>
            {busy ? 'Signing in…' : 'Sign in'}
          </button>
        </form>
        <p className="small faint">
          Accounts are managed by your installation administrator.
        </p>
      </div>
    </main>
  )
}
