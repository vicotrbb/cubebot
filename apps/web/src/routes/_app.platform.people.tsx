import { useQuery } from '@tanstack/react-query'
import { createFileRoute } from '@tanstack/react-router'
import { useState } from 'react'
import { admin } from '../api/admin'
import { Alert, Field, Skeleton, Status } from '../components/primitives'
import { useAdminAction } from '../features/platform/useAdminAction'
import '../features/platform/platform.css'

export const Route = createFileRoute('/_app/platform/people')({
  component: People,
})

function People() {
  const accounts = useQuery({ queryKey: ['accounts'], queryFn: admin.accounts })
  const invitations = useQuery({
    queryKey: ['invitations'],
    queryFn: admin.invitations,
  })
  const { run, error, notice, busy } = useAdminAction([
    'accounts',
    'invitations',
  ])
  const [token, setToken] = useState<{ login: string; token: string } | null>(
    null,
  )
  return (
    <div className="platform">
      <div>
        <p className="platform__lead">
          Accounts sign in with built-in credentials or, when configured, OpenID
          Connect. Suspending keeps the account's history and removes its
          access.
        </p>
        {accounts.isPending && <Skeleton lines={4} />}
        <table className="table">
          <thead>
            <tr>
              <th>Login</th>
              <th>Role</th>
              <th>State</th>
              <th />
            </tr>
          </thead>
          <tbody>
            {accounts.data?.map((account) => (
              <tr key={account.id}>
                <td>{account.login}</td>
                <td>{account.administrator ? 'Administrator' : 'Member'}</td>
                <td>{account.active ? 'Active' : 'Suspended'}</td>
                <td className="num">
                  {account.active && (
                    <button
                      type="button"
                      className="link-btn small"
                      disabled={busy}
                      onClick={() => {
                        if (window.confirm(`Suspend ${account.login}?`))
                          void run(
                            () => admin.suspend(account.id, account.revision),
                            'Suspended.',
                          )
                      }}
                    >
                      Suspend
                    </button>
                  )}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
        <h3 style={{ margin: '24px 0 8px' }}>Invitations</h3>
        <table className="table">
          <thead>
            <tr>
              <th>Login</th>
              <th>Status</th>
              <th>Expires</th>
              <th />
            </tr>
          </thead>
          <tbody>
            {invitations.data?.length === 0 && (
              <tr>
                <td colSpan={4} className="faint">
                  No invitations.
                </td>
              </tr>
            )}
            {invitations.data?.map((invitation) => (
              <tr key={invitation.id}>
                <td>{invitation.login}</td>
                <td>{invitation.status}</td>
                <td>
                  {new Date(invitation.expires_at_unix * 1000).toLocaleString()}
                </td>
                <td className="num">
                  {invitation.status === 'pending' && (
                    <button
                      type="button"
                      className="link-btn small"
                      disabled={busy}
                      onClick={() =>
                        void run(
                          () =>
                            admin.revokeInvitation(
                              invitation.id,
                              invitation.revision,
                            ),
                          'Revoked.',
                        )
                      }
                    >
                      Revoke
                    </button>
                  )}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      <aside className="platform__editor">
        <h2>Invite someone</h2>
        <form
          className="form"
          onSubmit={(event) => {
            event.preventDefault()
            const form = event.currentTarget
            const login = String(new FormData(form).get('login'))
            void run(() => admin.invite(login), 'Invitation created.').then(
              (made) => {
                if (made?.token) setToken({ login, token: made.token })
                form.reset()
              },
            )
          }}
        >
          <Field label="Login" hint="They choose a password when they accept">
            <input name="login" required maxLength={64} autoCapitalize="none" />
          </Field>
          <div className="actions">
            <button className="btn btn--primary btn--sm" disabled={busy}>
              Invite
            </button>
          </div>
        </form>
        {token && (
          <p className="notice notice--paint small" role="status">
            Token for {token.login}, shown only now. Send it with the accept
            link:
            <code
              style={{
                display: 'block',
                marginTop: 6,
                overflowWrap: 'anywhere',
              }}
            >
              {token.token}
            </code>
          </p>
        )}
        <h2>Create an account</h2>
        <form
          className="form"
          onSubmit={(event) => {
            event.preventDefault()
            const form = event.currentTarget
            const data = new FormData(form)
            void run(
              () =>
                admin.createAccount(
                  String(data.get('login')),
                  String(data.get('password')),
                  data.has('administrator'),
                ),
              'Account created.',
            ).then(() => form.reset())
          }}
        >
          <Field label="Login">
            <input
              name="login"
              required
              maxLength={64}
              autoCapitalize="none"
              autoComplete="off"
            />
          </Field>
          <Field label="Password">
            <input
              name="password"
              type="password"
              required
              autoComplete="new-password"
            />
          </Field>
          <label className="check">
            <input type="checkbox" name="administrator" /> Administrator
          </label>
          <div className="actions">
            <button className="btn btn--sm" disabled={busy}>
              Create
            </button>
          </div>
        </form>
        <Alert>{error}</Alert>
        <Status>{notice}</Status>
      </aside>
    </div>
  )
}
