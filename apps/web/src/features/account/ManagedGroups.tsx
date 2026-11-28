import { useQuery, useQueryClient } from '@tanstack/react-query'
import { UserMinus, UserPlus } from 'lucide-react'
import { useState } from 'react'
import { admin } from '../../api/admin'
import { failure } from '../../api/client'
import { useSession } from '../../app/queries'
import { PersonAvatar } from '../../components/PersonAvatar'
import { Alert, Status } from '../../components/primitives'
import '../platform/platform.css'

type Group = Awaited<ReturnType<typeof admin.groups>>[number]
type Person = Awaited<ReturnType<typeof admin.directory>>['people'][number]

const GROUPS_KEY = ['groups']

/** Members an administrator made group managers change who is in those groups here. */
export function ManagedGroups() {
  const session = useSession()
  // Only members can manage groups; guests have no directory to choose from.
  const member = session.data?.role === 'member'
  const groups = useQuery({
    queryKey: GROUPS_KEY,
    queryFn: admin.groups,
    enabled: Boolean(member),
  })
  const directory = useQuery({
    queryKey: ['directory'],
    queryFn: admin.directory,
    enabled: Boolean(member),
  })
  const managed = (groups.data ?? []).filter(
    (group) => group.active && group.source === 'local' && !group.role,
  )
  if (!member || managed.length === 0) return null
  return (
    <section className="profile__about" aria-labelledby="managed-groups">
      <div>
        <h3 id="managed-groups">Groups you manage</h3>
        <p className="faint small">
          Everyone in a group gets what is shared with it. An administrator made
          you a manager of these.
        </p>
      </div>
      {managed.map((group) => (
        <GroupMembers
          key={group.id}
          group={group}
          people={directory.data?.people ?? []}
        />
      ))}
    </section>
  )
}

function GroupMembers({ group, people }: { group: Group; people: Person[] }) {
  const client = useQueryClient()
  const [chosen, setChosen] = useState('')
  const [error, setError] = useState('')
  const [notice, setNotice] = useState('')
  const [busy, setBusy] = useState(false)
  const name = (id: string) => {
    const person = people.find((candidate) => candidate.id === id)
    return person?.name ?? person?.login ?? 'Someone'
  }
  const change = async (account: string, present: boolean) => {
    setBusy(true)
    setError('')
    try {
      await admin.membership(group.id, account, group.revision, present)
      await client.invalidateQueries({ queryKey: GROUPS_KEY })
      setNotice(
        `${name(account)} ${present ? 'joined' : 'left'} ${group.name}.`,
      )
      setChosen('')
    } catch (failed) {
      setError(failure(failed))
    } finally {
      setBusy(false)
    }
  }
  const outside = people.filter((person) => !group.members.includes(person.id))
  return (
    <div className="you__part">
      <h4>{group.name}</h4>
      <ul className="access__rows">
        {group.members.map((id) => (
          <li key={id} className="access__row">
            <PersonAvatar account={id} name={name(id)} />
            <span className="access__target">
              <span className="access__name">{name(id)}</span>
              {group.managers.includes(id) && (
                <span className="small faint">Manager</span>
              )}
            </span>
            {!group.managers.includes(id) && (
              <button
                type="button"
                className="btn btn--ghost btn--sm"
                disabled={busy}
                onClick={() => void change(id, false)}
              >
                <UserMinus aria-hidden="true" />
                Remove
              </button>
            )}
          </li>
        ))}
      </ul>
      {outside.length > 0 && (
        <form
          className="you__row"
          onSubmit={(event) => {
            event.preventDefault()
            if (chosen) void change(chosen, true)
          }}
        >
          <select
            className="input"
            aria-label={`Add someone to ${group.name}`}
            value={chosen}
            onChange={(event) => setChosen(event.target.value)}
          >
            <option value="" disabled>
              Add someone…
            </option>
            {outside.map((person) => (
              <option key={person.id} value={person.id}>
                {person.name ?? person.login}
              </option>
            ))}
          </select>
          <button className="btn btn--sm" disabled={busy || !chosen}>
            <UserPlus aria-hidden="true" />
            Add
          </button>
        </form>
      )}
      <Alert>{error}</Alert>
      <Status>{notice}</Status>
    </div>
  )
}
