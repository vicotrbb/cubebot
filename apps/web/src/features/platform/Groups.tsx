import { useQuery } from '@tanstack/react-query'
import { Link } from '@tanstack/react-router'
import { KeyRound, UserCog, Users, X } from 'lucide-react'
import { useState } from 'react'
import { logos } from '../../../../../modules/brand/logos'
import { admin, type Role } from '../../api/admin'
import { api, ok } from '../../api/client'
import { PersonAvatar } from '../../components/PersonAvatar'
import {
  Alert,
  confirmAction,
  Dialog,
  Field,
  Skeleton,
  when,
} from '../../components/primitives'
import { roleLabel, rolesFor } from './roles'
import type { useAdminAction } from './useAdminAction'

export type Group = Awaited<ReturnType<typeof admin.groups>>[number]
type Account = Awaited<ReturnType<typeof admin.accounts>>[number]
type Action = ReturnType<typeof useAdminAction>

/** A group's mark: Google's own for a mirrored Google group. */
export function GroupMark({ source }: { source: string }) {
  const google = logos.google
  return (
    <span className="access__mark">
      {source === 'google' && google ? (
        // Static mark from our own source, never user content.
        <svg
          viewBox={google.viewBox}
          aria-hidden="true"
          dangerouslySetInnerHTML={{ __html: google.svg }}
        />
      ) : source === 'oidc' ? (
        <KeyRound aria-hidden="true" />
      ) : (
        <Users aria-hidden="true" />
      )}
    </span>
  )
}

const synced = (group: Group) =>
  group.synced_at_unix
    ? `synced ${when(new Date(group.synced_at_unix * 1000).toISOString())}`
    : 'not synced yet'

export function GroupOrigin({ group }: { group: Group }) {
  if (group.source === 'local') return null
  const what =
    group.source === 'google'
      ? `Google group ${group.external_id ?? ''}`
      : `Single sign-on group “${group.external_id ?? ''}”`
  return (
    <span className="small faint">
      {what}, {synced(group)}
    </span>
  )
}

export function GroupRole({
  group,
  owner,
  writer,
  action,
}: {
  group: Group
  owner: boolean
  writer: boolean
  action: Action
}) {
  const locked = !owner && group.role === 'administrator'
  return (
    <section className="access__section">
      <h3>Role</h3>
      {writer && !locked ? (
        <select
          className="input access__level groups__role"
          aria-label={`Role ${group.name} gives`}
          value={group.role ?? ''}
          disabled={action.busy}
          onChange={(event) =>
            void setRole(
              group,
              (event.target.value || null) as Role | null,
              action,
            )
          }
        >
          <option value="">No role</option>
          {rolesFor(owner).map((role) => (
            <option key={role.id} value={role.id}>
              {role.label}
            </option>
          ))}
        </select>
      ) : (
        <p>{group.role ? roleLabel(group.role) : 'No role'}</p>
      )}
      <p className="small faint">
        {group.role
          ? 'Everyone in it has this role, which nobody can change by hand while they are in the group. In several groups, the strongest role wins.'
          : 'Members keep their own roles. Give the group a role to set everyone’s at once.'}
        {locked &&
          ' Only the installation owner changes an administrator group.'}
      </p>
    </section>
  )
}

async function setRole(group: Group, role: Role | null, action: Action) {
  const yes = await confirmAction({
    title: role
      ? `Make everyone in ${group.name} ${roleLabel(role).toLowerCase()}?`
      : `Stop setting a role for ${group.name}?`,
    body: role
      ? `All ${group.members.length} members, and whoever joins later, get this role until they leave the group.`
      : 'Members whose role came from this group become members, unless another group sets one.',
    action: role ? 'Set role' : 'Stop setting it',
  })
  if (yes)
    await action.run(
      () => admin.groupRole(group.id, role, group.revision),
      role
        ? `${group.name} sets ${roleLabel(role).toLowerCase()}.`
        : `${group.name} no longer sets a role.`,
    )
}

export function GroupMembers({
  group,
  people,
  writer,
  action,
}: {
  group: Group
  people: Account[]
  writer: boolean
  action: Action
}) {
  const editable = writer && group.source === 'local'
  const inGroup = people.filter((person) => group.members.includes(person.id))
  const others = people.filter((person) => !group.members.includes(person.id))
  const change = (account: Account, present: boolean, manager?: boolean) =>
    void action.run(
      () =>
        admin.membership(
          group.id,
          account.id,
          group.revision,
          present,
          manager,
        ),
      present
        ? `${account.login} is in ${group.name}.`
        : `${account.login} left ${group.name}.`,
    )
  return (
    <section className="access__section">
      <h3>Members</h3>
      <ul className="access__members">
        {inGroup.map((person) => {
          const manager = group.managers.includes(person.id)
          return (
            <li key={person.id}>
              <PersonAvatar account={person.id} name={person.login} />
              {person.login}
              {!editable && manager && <span className="faint">manager</span>}
              {editable && (
                <>
                  <button
                    type="button"
                    className="access__remove groups__manager"
                    aria-pressed={manager}
                    aria-label={`${person.login} manages ${group.name}`}
                    title={
                      manager
                        ? 'Manager: may add and remove members'
                        : 'Make a manager'
                    }
                    disabled={action.busy}
                    onClick={() => change(person, true, !manager)}
                  >
                    <UserCog />
                  </button>
                  <button
                    type="button"
                    className="access__remove"
                    aria-label={`Remove ${person.login} from ${group.name}`}
                    disabled={action.busy}
                    onClick={() => change(person, false)}
                  >
                    <X />
                  </button>
                </>
              )}
            </li>
          )
        })}
        {editable && others.length > 0 && (
          <li className="access__add">
            <select
              className="input"
              aria-label={`Add someone to ${group.name}`}
              value=""
              disabled={action.busy}
              onChange={(event) => {
                const person = others.find(
                  (item) => item.id === event.target.value,
                )
                if (person) change(person, true)
              }}
            >
              <option value="" disabled>
                Add someone…
              </option>
              {others.map((person) => (
                <option key={person.id} value={person.id}>
                  {person.login}
                </option>
              ))}
            </select>
          </li>
        )}
      </ul>
      <MembersNote group={group} empty={inGroup.length === 0} />
    </section>
  )
}

function MembersNote({ group, empty }: { group: Group; empty: boolean }) {
  if (group.source === 'google')
    return (
      <p className="small faint">
        Members follow the Google group: people whose single sign-on email is in
        it. Changes arrive with each sync, every 15 minutes.
      </p>
    )
  if (group.source === 'oidc')
    return (
      <p className="small faint">
        Members follow your identity provider: everyone whose groups claim lists
        “{group.external_id}” joins when they sign in, and leaves when it stops
        listing it.
      </p>
    )
  return (
    <p className="small faint">
      {empty
        ? 'Nobody yet. What you allow below reaches everyone you add. '
        : ''}
      Managers may add and remove members themselves, unless the group sets a
      role.
    </p>
  )
}

export function LinkGroups({
  groups,
  action,
}: {
  groups: Group[]
  action: Action
}) {
  const [open, setOpen] = useState<'google' | 'oidc' | null>(null)
  const close = () => setOpen(null)
  return (
    <div className="groups__link">
      <button
        type="button"
        className="link-btn small"
        onClick={() => setOpen('google')}
      >
        Link a Google group
      </button>
      <button
        type="button"
        className="link-btn small"
        onClick={() => setOpen('oidc')}
      >
        Link a single sign-on group
      </button>
      <Dialog
        open={open === 'google'}
        title="Link a Google group"
        onClose={close}
      >
        <GoogleGroups groups={groups} action={action} onDone={close} />
      </Dialog>
      <Dialog
        open={open === 'oidc'}
        title="Link a single sign-on group"
        onClose={close}
      >
        <ClaimGroup action={action} onDone={close} />
      </Dialog>
    </div>
  )
}

function GoogleGroups({
  groups,
  action,
  onDone,
}: {
  groups: Group[]
  action: Action
  onDone: () => void
}) {
  const workspace = useQuery({
    queryKey: ['workspace-groups'],
    queryFn: admin.workspaceGroups,
    retry: false,
  })
  const [search, setSearch] = useState('')
  const linked = new Set(
    groups
      .filter((group) => group.source === 'google')
      .map((group) => group.external_id),
  )
  if (workspace.isPending) return <Skeleton lines={4} />
  if (workspace.isError)
    return (
      <p className="muted">
        Google Workspace is not set up, or Google refused to list its groups.
        Check it under <Link to="/platform/sign-in">Sign-in</Link>.
      </p>
    )
  const term = search.trim().toLowerCase()
  const shown = workspace.data.filter((item) =>
    `${item.name} ${item.email}`.toLowerCase().includes(term),
  )
  const link = (item: (typeof shown)[number]) =>
    void action
      .run(async () => {
        const made = await admin.createGroup(item.name, {
          source: 'google',
          external_id: item.email,
        })
        // Fills it now instead of at the next sync; a failed sync shows under Sign-in.
        await admin.syncGoogle().catch(() => undefined)
        return made
      }, `${item.name} is linked.`)
      .then((made) => {
        if (made) onDone()
      })
  return (
    <div className="form">
      <p className="small muted">
        Its members are mirrored here and kept in step every 15 minutes. People
        are matched by the verified email of their single sign-on identity.
      </p>
      <Alert>{action.error}</Alert>
      <input
        type="search"
        className="input"
        aria-label="Filter Google groups"
        placeholder="Filter"
        value={search}
        onChange={(event) => setSearch(event.target.value)}
      />
      <ul className="access__rows groups__pick">
        {shown.map((item) => (
          <li key={item.email} className="access__row">
            <span className="access__target">
              <span className="access__name">{item.name}</span>
              <span className="small faint">
                {item.email} · {item.direct_members_count}{' '}
                {item.direct_members_count === 1 ? 'member' : 'members'}
              </span>
            </span>
            {linked.has(item.email) ? (
              <span className="small faint">Linked</span>
            ) : (
              <button
                type="button"
                className="btn btn--sm"
                disabled={action.busy}
                onClick={() => link(item)}
              >
                Link
              </button>
            )}
          </li>
        ))}
      </ul>
      {shown.length === 0 && (
        <p className="small faint">No Google group matches that.</p>
      )}
    </div>
  )
}

function ClaimGroup({
  action,
  onDone,
}: {
  action: Action
  onDone: () => void
}) {
  const sso = useQuery({
    queryKey: ['single-sign-on'],
    queryFn: () => ok(api.GET('/api/identity/oidc/settings')),
  })
  const claim = sso.data?.groups_claim
  return (
    <form
      className="form"
      onSubmit={(event) => {
        event.preventDefault()
        const data = new FormData(event.currentTarget)
        const name = String(data.get('name') ?? '').trim()
        const value = String(data.get('value') ?? '').trim()
        void action
          .run(
            () =>
              admin.createGroup(name, { source: 'oidc', external_id: value }),
            `${name} is linked.`,
          )
          .then((made) => {
            if (made) onDone()
          })
      }}
    >
      <p className="small muted">
        {claim
          ? `Everyone whose “${claim}” claim lists this value joins when they sign in with single sign-on.`
          : 'Name the groups claim under Sign-in first; until then no sign-in fills this group.'}
      </p>
      <Alert>{action.error}</Alert>
      <Field label="Name" hint="What people see here">
        <input name="name" required maxLength={128} />
      </Field>
      <Field
        label="Claim value"
        hint="Exactly as your identity provider sends it"
      >
        <input name="value" required maxLength={255} autoCapitalize="none" />
      </Field>
      <div className="actions dialog__actions">
        <button className="btn btn--primary" disabled={action.busy}>
          Link group
        </button>
      </div>
    </form>
  )
}
