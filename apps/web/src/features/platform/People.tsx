import { useQuery } from '@tanstack/react-query'
import {
  ArrowRightLeft,
  Crown,
  KeyRound,
  MoreHorizontal,
  ShieldOff,
  UserCheck,
  UserX,
} from 'lucide-react'
import { useState } from 'react'
import { admin, type Role } from '../../api/admin'
import { ApiError } from '../../api/client'
import { PersonAvatar } from '../../components/PersonAvatar'
import {
  Alert,
  confirmAction,
  CopyButton,
  Dialog,
  Field,
  Menu,
  Skeleton,
  Status,
  when,
} from '../../components/primitives'
import {
  heirs,
  lockOf,
  resetLink,
  roleLabel,
  roleLockOf,
  rolesFor,
} from './roles'
import { Invitations, NewPeople } from './PeopleForms'
import { MemberPolicy } from './Policy'
import { useAdminAction } from './useAdminAction'
import { useViewer } from './useViewer'
import './platform.css'

export type Account = Awaited<ReturnType<typeof admin.accounts>>[number]
type Group = Awaited<ReturnType<typeof admin.groups>>[number]
type Viewer = ReturnType<typeof useViewer>
export type Action = ReturnType<typeof useAdminAction>

export function PlatformPeople() {
  const accounts = useQuery({ queryKey: ['accounts'], queryFn: admin.accounts })
  const groups = useQuery({ queryKey: ['groups'], queryFn: admin.groups })
  const viewer = useViewer()
  const action = useAdminAction(['accounts', 'invitations', 'standing'])
  return (
    <div className={viewer.writer ? 'platform' : 'platform platform--wide'}>
      <div>
        <p className="platform__lead">
          Everyone with an account. Suspending someone signs them out and keeps
          their history; give what they own to someone else so their agents,
          rooms and teams keep a person in charge.
        </p>
        <Alert>{action.error}</Alert>
        <Status>{action.notice}</Status>
        {accounts.isPending && <Skeleton lines={4} />}
        {accounts.isError && <Alert>People could not be loaded.</Alert>}
        {accounts.data && (
          <PeopleTable
            people={accounts.data}
            groups={groups.data ?? []}
            viewer={viewer}
            action={action}
          />
        )}
        <Invitations writer={viewer.writer} action={action} />
        <MemberPolicy writer={viewer.writer} />
      </div>
      {viewer.writer && <NewPeople owner={viewer.owner} action={action} />}
    </div>
  )
}

function PeopleTable({
  people,
  groups,
  viewer,
  action,
}: {
  people: Account[]
  groups: Group[]
  viewer: Viewer
  action: Action
}) {
  const [link, setLink] = useState<{ login: string; url: string } | null>(null)
  const [moving, setMoving] = useState<Account | null>(null)
  return (
    <>
      <table className="table people">
        <thead>
          <tr>
            <th>Person</th>
            <th>Role</th>
            <th>Two-factor</th>
            <th>Last seen</th>
            <th className="num">Owns</th>
            <th>
              <span className="sr-only">Actions</span>
            </th>
          </tr>
        </thead>
        <tbody>
          {people.map((person) => (
            <tr key={person.id}>
              <PersonCell person={person} self={person.id === viewer.id} />
              <td>
                <RoleCell
                  person={person}
                  groups={groups}
                  viewer={viewer}
                  action={action}
                />
              </td>
              <td>
                {person.two_factor ? 'On' : <span className="faint">Off</span>}
              </td>
              <SeenCell person={person} />
              <td className="num">{person.owned}</td>
              <td className="num">
                {viewer.writer && !lockOf(person, viewer) && (
                  <PersonMenu
                    person={person}
                    viewer={viewer}
                    action={action}
                    onLink={setLink}
                    onMove={setMoving}
                  />
                )}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
      <Dialog
        open={link !== null}
        title={`Password reset link for ${link?.login ?? ''}`}
        onClose={() => setLink(null)}
      >
        {link && <ResetLink url={link.url} />}
      </Dialog>
      <Dialog
        open={moving !== null}
        title={`Give away what ${moving?.login ?? ''} owns`}
        onClose={() => setMoving(null)}
      >
        {moving && (
          <TransferAll
            person={moving}
            people={people}
            action={action}
            onDone={() => setMoving(null)}
          />
        )}
      </Dialog>
    </>
  )
}

function PersonCell({ person, self }: { person: Account; self: boolean }) {
  return (
    <td>
      <div className="people__who">
        <PersonAvatar account={person.id} name={person.login} />
        <span className="people__name">
          <span>
            {person.login}
            {self && <span className="faint"> (you)</span>}
          </span>
          {person.name && <span className="small faint">{person.name}</span>}
        </span>
        {person.owner && (
          <span className="chip" title="The installation owner">
            <Crown aria-hidden="true" /> Owner
          </span>
        )}
        {!person.active && <span className="chip chip--danger">Suspended</span>}
      </div>
    </td>
  )
}

function SeenCell({ person }: { person: Account }) {
  const seen = person.last_seen_at_unix
  return (
    <td>
      {seen ? (
        <span title={new Date(seen * 1000).toLocaleString()}>
          {when(new Date(seen * 1000).toISOString())}
        </span>
      ) : (
        <span className="faint">Never</span>
      )}
      {person.sessions > 0 && (
        <span className="small faint tabular people__sub">
          {person.sessions} open{' '}
          {person.sessions === 1 ? 'session' : 'sessions'}
        </span>
      )}
    </td>
  )
}

function RoleCell({
  person,
  groups,
  viewer,
  action,
}: {
  person: Account
  groups: Group[]
  viewer: Viewer
  action: Action
}) {
  const lock = roleLockOf(person, viewer, groups)
  if (!viewer.writer || lock)
    return (
      <>
        {roleLabel(person.role)}
        {viewer.writer && lock && lock !== 'You' && (
          <span className="small faint people__sub">{lock}</span>
        )}
      </>
    )
  return (
    <select
      className="input people__role"
      aria-label={`Role of ${person.login}`}
      value={person.role}
      disabled={action.busy}
      onChange={(event) =>
        void changeRole(person, event.target.value as Role, action)
      }
    >
      {rolesFor(viewer.owner).map((role) => (
        <option key={role.id} value={role.id}>
          {role.label}
        </option>
      ))}
    </select>
  )
}

async function changeRole(person: Account, role: Role, action: Action) {
  const admins = role === 'administrator' || person.role === 'administrator'
  if (
    admins &&
    !(await confirmAction({
      title:
        role === 'administrator'
          ? `Make ${person.login} an administrator?`
          : `Make ${person.login} ${roleLabel(role).toLowerCase()}?`,
      body:
        role === 'administrator'
          ? 'They can change everything on this installation, its people and every agent and room. Only you, the owner, can undo it.'
          : 'They stop being an administrator and keep only what their new role and their grants allow.',
      action: role === 'administrator' ? 'Make administrator' : 'Change role',
    }))
  )
    return
  await action.run(
    async () => {
      try {
        return await admin.changeRole(person.id, role, person.revision)
      } catch (caught) {
        if (caught instanceof ApiError && caught.status === 409)
          throw new Error(
            `${person.login}’s role is set by a group, or changed since you opened this page. Refresh and check their groups.`,
            { cause: caught },
          )
        throw caught
      }
    },
    `${person.login} is now ${roleLabel(role).toLowerCase()}.`,
  )
}

function PersonMenu({
  person,
  viewer,
  action,
  onLink,
  onMove,
}: {
  person: Account
  viewer: Viewer
  action: Action
  onLink: (link: { login: string; url: string }) => void
  onMove: (person: Account) => void
}) {
  const heir = viewer.owner && person.active && person.role === 'administrator'
  return (
    <Menu
      className="menu--right"
      trigger={<MoreHorizontal />}
      triggerClassName="btn btn--ghost btn--icon btn--sm"
      label={`Actions for ${person.login}`}
    >
      {person.active && (
        <button
          type="button"
          className="menu__item"
          onClick={() =>
            void action
              .run(() => admin.passwordReset(person.id), 'Link made.')
              .then((made) => {
                if (made)
                  onLink({
                    login: person.login,
                    url: resetLink(location.origin, made.token),
                  })
              })
          }
        >
          <KeyRound /> Make a password reset link
        </button>
      )}
      {person.two_factor && (
        <button
          type="button"
          className="menu__item"
          onClick={() => void resetTwoFactor(person, action)}
        >
          <ShieldOff /> Reset two-factor
        </button>
      )}
      {person.owned > 0 && (
        <button
          type="button"
          className="menu__item"
          onClick={() => onMove(person)}
        >
          <ArrowRightLeft /> Give everything they own to…
        </button>
      )}
      {heir && (
        <button
          type="button"
          className="menu__item"
          onClick={() => void handOver(person, action)}
        >
          <Crown /> Make installation owner
        </button>
      )}
      <SuspendItem person={person} action={action} />
    </Menu>
  )
}

function SuspendItem({ person, action }: { person: Account; action: Action }) {
  if (!person.active)
    return (
      <button
        type="button"
        className="menu__item"
        onClick={() =>
          void action.run(
            () => admin.reactivate(person.id, person.revision),
            `${person.login} can sign in again.`,
          )
        }
      >
        <UserCheck /> Reactivate
      </button>
    )
  return (
    <button
      type="button"
      className="menu__item menu__item--danger"
      onClick={async () => {
        const owned =
          person.owned > 0
            ? ` What they own keeps working and shows under Ownership as needing a new owner; you can give it all to someone first.`
            : ''
        const yes = await confirmAction({
          title: `Suspend ${person.login}?`,
          body: `They are signed out now and cannot sign in. Their history stays.${owned}`,
          action: 'Suspend',
        })
        if (yes)
          void action.run(
            () => admin.suspend(person.id, person.revision),
            `${person.login} is suspended.`,
          )
      }}
    >
      <UserX /> Suspend
    </button>
  )
}

async function resetTwoFactor(person: Account, action: Action) {
  const yes = await confirmAction({
    title: `Reset two-factor for ${person.login}?`,
    body: 'For someone who lost their authenticator. They sign in with their password alone until they set up a new one.',
    action: 'Reset two-factor',
  })
  if (yes)
    await action.run(
      () => admin.resetTwoFactor(person.id),
      `${person.login} signs in without a code until they set one up again.`,
    )
}

async function handOver(person: Account, action: Action) {
  const yes = await confirmAction({
    title: `Make ${person.login} the installation owner?`,
    body: `You stay an administrator, but only ${person.login} can then make or unmake administrators, including you. Only they can hand it back.`,
    action: 'Hand over ownership',
  })
  if (yes)
    await action.run(
      () => admin.handOver(person.id),
      `${person.login} owns the installation now.`,
    )
}

function ResetLink({ url }: { url: string }) {
  return (
    <div className="form">
      <p className="small muted">
        Shown only now. Send it to them privately: it works once, for one day,
        and setting a new password with it signs them out everywhere else.
      </p>
      <div className="sso__callback">
        <code>{url}</code>
        <CopyButton text={url} label="Copy reset link" />
      </div>
    </div>
  )
}

function TransferAll({
  person,
  people,
  action,
  onDone,
}: {
  person: Account
  people: Account[]
  action: Action
  onDone: () => void
}) {
  const choices = heirs(people, person.id)
  const [to, setTo] = useState(choices[0]?.id ?? '')
  const heir = choices.find((item) => item.id === to)
  return (
    <form
      className="form"
      onSubmit={(event) => {
        event.preventDefault()
        if (!heir) return
        void action
          .run(
            () => admin.transferAll(person.id, heir.id),
            `${heir.login} owns what ${person.login} owned.`,
          )
          .then((done) => {
            if (done) onDone()
          })
      }}
    >
      <p className="small muted">
        {person.login} owns {person.owned} live{' '}
        {person.owned === 1 ? 'agent, room or team' : 'agents, rooms and teams'}
        . They all move to the person you pick, and who each is shared with
        stays as it is. Personal connections stay with {person.login}: they hold
        their own credentials.
      </p>
      <Alert>{action.error}</Alert>
      {choices.length === 0 ? (
        <p className="small faint">
          Nobody else can own things: only active administrators and members
          can.
        </p>
      ) : (
        <Field label="Give to" hint="Active administrators and members">
          <select value={to} onChange={(event) => setTo(event.target.value)}>
            {choices.map((item) => (
              <option key={item.id} value={item.id}>
                {item.login}
              </option>
            ))}
          </select>
        </Field>
      )}
      <div className="actions dialog__actions">
        <button className="btn btn--primary" disabled={!heir || action.busy}>
          {heir ? `Give everything to ${heir.login}` : 'Give everything'}
        </button>
      </div>
    </form>
  )
}
