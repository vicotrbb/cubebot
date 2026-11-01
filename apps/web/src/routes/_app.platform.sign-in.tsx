import { useQuery } from '@tanstack/react-query'
import { createFileRoute } from '@tanstack/react-router'
import { admin } from '../api/admin'
import { Alert, Skeleton, Status } from '../components/primitives'
import { useAdminAction } from '../features/platform/useAdminAction'
import '../features/platform/platform.css'

export const Route = createFileRoute('/_app/platform/sign-in')({
  component: SignInSettings,
})

function SignInSettings() {
  const bindings = useQuery({ queryKey: ['bindings'], queryFn: admin.bindings })
  const { run, error, notice, busy } = useAdminAction(['bindings'])
  return (
    <div className="platform">
      <div>
        <p className="platform__lead">
          OpenID Connect is configured on the orchestrator (ADR 0050). Bindings
          tie an account to an issuer and subject; revoking one ends its OIDC
          sessions and keeps the record as a tombstone.
        </p>
        {bindings.isPending && <Skeleton lines={3} />}
        {bindings.isError && (
          <Alert>
            OpenID Connect is not configured, or bindings could not be loaded.
          </Alert>
        )}
        <table className="table">
          <thead>
            <tr>
              <th>Account</th>
              <th>Issuer</th>
              <th>Subject</th>
              <th>State</th>
              <th />
            </tr>
          </thead>
          <tbody>
            {bindings.data?.length === 0 && (
              <tr>
                <td colSpan={5} className="faint">
                  No bindings yet.
                </td>
              </tr>
            )}
            {bindings.data?.map((binding) => (
              <tr key={binding.id}>
                <td>
                  <code>{binding.account_id.slice(0, 8)}</code>
                </td>
                <td>{binding.issuer}</td>
                <td>
                  <code>{binding.subject}</code>
                </td>
                <td>{binding.active ? 'Active' : 'Revoked'}</td>
                <td className="num">
                  {binding.active && (
                    <button
                      type="button"
                      className="link-btn small"
                      disabled={busy}
                      onClick={() =>
                        void run(
                          () =>
                            admin.revokeBinding(binding.id, binding.revision),
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
        <Alert>{error}</Alert>
        <Status>{notice}</Status>
      </div>
      <aside className="platform__editor">
        <h2>Built-in sign-in</h2>
        <p className="small muted">
          Built-in credentials always work for installation recovery. Password
          checks are rate limited per account; see the operator reference for
          the limits.
        </p>
      </aside>
    </div>
  )
}
