import { useQuery } from '@tanstack/react-query'
import { useState } from 'react'
import { admin, type Role } from '../../api/admin'
import { CopyButton, Field } from '../../components/primitives'
import type { Action } from './People'
import { roleLabel, ROLES, rolesFor } from './roles'

const INVITED = ROLES.filter((role) => role.id !== 'administrator')

export function Invitations({
  writer,
  action,
}: {
  writer: boolean
  action: Action
}) {
  const invitations = useQuery({
    queryKey: ['invitations'],
    queryFn: admin.invitations,
  })
  return (
    <>
      <h2 className="platform__h2">Invitations</h2>
      <table className="table">
        <thead>
          <tr>
            <th>Login</th>
            <th>Role</th>
            <th>Status</th>
            <th>Expires</th>
            <th>
              <span className="sr-only">Actions</span>
            </th>
          </tr>
        </thead>
        <tbody>
          {invitations.data?.length === 0 && (
            <tr>
              <td colSpan={5} className="faint">
                No invitations.
              </td>
            </tr>
          )}
          {invitations.data?.map((invitation) => (
            <tr key={invitation.id}>
              <td>{invitation.login}</td>
              <td>{roleLabel(invitation.role)}</td>
              <td>{invitation.status}</td>
              <td className="tabular">
                {new Date(invitation.expires_at_unix * 1000).toLocaleString()}
              </td>
              <td className="num">
                {writer && invitation.status === 'pending' && (
                  <button
                    type="button"
                    className="link-btn small"
                    disabled={action.busy}
                    onClick={() =>
                      void action.run(
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
    </>
  )
}

function RoleSelect({ roles, hint }: { roles: typeof ROLES; hint?: string }) {
  return (
    <Field label="Role" hint={hint}>
      <select name="role" defaultValue="member">
        {roles.map((role) => (
          <option key={role.id} value={role.id}>
            {role.label}
          </option>
        ))}
      </select>
    </Field>
  )
}

export function NewPeople({
  owner,
  action,
}: {
  owner: boolean
  action: Action
}) {
  const [token, setToken] = useState<{ login: string; token: string } | null>(
    null,
  )
  return (
    <aside className="platform__editor">
      <h2>Invite someone</h2>
      <form
        className="form"
        onSubmit={(event) => {
          event.preventDefault()
          const form = event.currentTarget
          const data = new FormData(form)
          const login = String(data.get('login'))
          const role = String(data.get('role')) as Role
          void action
            .run(() => admin.invite(login, role), 'Invitation created.')
            .then((made) => {
              if (made?.token) setToken({ login, token: made.token })
              form.reset()
            })
        }}
      >
        <Field label="Login" hint="They choose a password when they accept">
          <input name="login" required maxLength={64} autoCapitalize="none" />
        </Field>
        <RoleSelect
          roles={INVITED}
          hint="Make them an administrator once they join"
        />
        <div className="actions">
          <button className="btn btn--primary btn--sm" disabled={action.busy}>
            Invite
          </button>
        </div>
      </form>
      {token && (
        <div className="notice notice--paint small" role="status">
          Token for {token.login}, shown only now. Send it with the accept link.
          <div className="sso__callback people__token">
            <code>{token.token}</code>
            <CopyButton text={token.token} label="Copy invitation token" />
          </div>
        </div>
      )}
      <h2>Create an account</h2>
      <form
        className="form"
        onSubmit={(event) => {
          event.preventDefault()
          const form = event.currentTarget
          const data = new FormData(form)
          void action
            .run(
              () =>
                admin.createAccount(
                  String(data.get('login')),
                  String(data.get('password')),
                  String(data.get('role')) as Role,
                ),
              'Account created.',
            )
            .then((made) => {
              if (made) form.reset()
            })
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
        <RoleSelect
          roles={rolesFor(owner)}
          {...(owner ? {} : { hint: 'Only the owner creates administrators' })}
        />
        <div className="actions">
          <button className="btn btn--sm" disabled={action.busy}>
            Create
          </button>
        </div>
      </form>
    </aside>
  )
}
