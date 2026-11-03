import { createFileRoute, useNavigate } from '@tanstack/react-router'
import {
  signOut,
  useInstallation,
  usePreferences,
  useSavePreferences,
  useSession,
} from '../app/queries'
import { applyTheme, type Theme } from '../app/theme'
import { Field } from '../components/primitives'

export const Route = createFileRoute('/_app/you')({ component: You })

function You() {
  const session = useSession()
  const installation = useInstallation()
  const preferences = usePreferences()
  const save = useSavePreferences()
  const navigate = useNavigate()
  const theme = preferences.data?.theme ?? 'system'
  return (
    <div className="page">
      <header className="page__head">
        <h1>You</h1>
      </header>
      <div className="page__body page__body--narrow">
        <div className="form">
          <dl className="kv">
            <dt>Login</dt>
            <dd>{session.data?.login ?? session.data?.id}</dd>
            <dt>Role</dt>
            <dd>{session.data?.administrator ? 'Administrator' : 'Member'}</dd>
            <dt>Installation</dt>
            <dd>
              {installation.data?.name} ·{' '}
              {installation.data?.controller.replaceAll('_', ' ')}
            </dd>
          </dl>
          <Field label="Theme">
            <select
              value={theme}
              onChange={(event) => {
                const next = event.target.value as Theme
                applyTheme(next)
                if (preferences.data)
                  save.mutate({ ...preferences.data, theme: next })
              }}
            >
              <option value="system">Match the system</option>
              <option value="light">Light</option>
              <option value="dark">Dark</option>
            </select>
          </Field>
          <div className="actions">
            <button
              type="button"
              className="btn"
              onClick={async () => {
                await signOut()
                await navigate({ to: '/sign-in' })
              }}
            >
              Sign out
            </button>
          </div>
        </div>
      </div>
    </div>
  )
}
