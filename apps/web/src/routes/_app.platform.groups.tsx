import { useQuery } from '@tanstack/react-query'
import { createFileRoute } from '@tanstack/react-router'
import { Hash, Plus, Search, Shield, Users, X } from 'lucide-react'
import { useState } from 'react'
import { admin } from '../api/admin'
import type { AgentSummary, RoomSummary } from '../api/types'
import { useAgents, useRooms } from '../app/queries'
import { Cube } from '../components/Cube'
import { PersonAvatar } from '../components/PersonAvatar'
import {
  Alert,
  confirmAction,
  Skeleton,
  Status,
} from '../components/primitives'
import {
  grantsOn,
  LABEL,
  levelOf,
  LEVELS,
  MEANING,
  type Grant,
  type Level,
  type Target,
} from '../features/platform/access'
import { applyLevel } from '../features/platform/applyLevel'
import { useAdminAction } from '../features/platform/useAdminAction'
import '../features/platform/platform.css'

export const Route = createFileRoute('/_app/platform/groups')({
  component: GroupsAndAccess,
})

type Group = Awaited<ReturnType<typeof admin.groups>>[number]
type Account = Awaited<ReturnType<typeof admin.accounts>>[number]
type Chosen = { kind: 'group' | 'user'; id: string }

const active = <T extends { active: boolean }>(list: T[] | undefined) =>
  list?.filter((item) => item.active) ?? []

function GroupsAndAccess() {
  const groups = useQuery({ queryKey: ['groups'], queryFn: admin.groups })
  const accounts = useQuery({ queryKey: ['accounts'], queryFn: admin.accounts })
  const grants = useQuery({ queryKey: ['grants'], queryFn: admin.grants })
  const action = useAdminAction(['groups', 'grants', 'access-events'])
  const [chosen, setChosen] = useState<Chosen | null>(null)
  const activeGroups = active(groups.data)
  const people = active(accounts.data)
  const selected = chosen ?? firstChoice(activeGroups, people)
  const group =
    selected?.kind === 'group'
      ? activeGroups.find((item) => item.id === selected.id)
      : undefined
  const person =
    selected?.kind === 'user'
      ? people.find((item) => item.id === selected.id)
      : undefined
  return (
    <div className="access">
      <Recipients
        groups={activeGroups}
        people={people}
        selected={selected}
        action={action}
        onChoose={setChosen}
      />
      <div className="access__detail">
        <Alert>{action.error}</Alert>
        <Status>{action.notice}</Status>
        {(groups.isPending || accounts.isPending) && <Skeleton lines={4} />}
        {group && (
          <GroupDetail
            group={group}
            people={people}
            grants={grants.data ?? []}
            action={action}
          />
        )}
        {person && (
          <PersonDetail
            person={person}
            groups={activeGroups}
            grants={grants.data ?? []}
            action={action}
          />
        )}
      </div>
    </div>
  )
}

function Recipients({
  groups,
  people,
  selected,
  action,
  onChoose,
}: {
  groups: Group[]
  people: Account[]
  selected: Chosen | null
  action: Action
  onChoose: (chosen: Chosen) => void
}) {
  return (
    <nav className="access__list" aria-label="People and groups">
      <div className="access__group">
        <h2>Groups</h2>
        {groups.map((item) => (
          <button
            key={item.id}
            type="button"
            className="access__pick"
            aria-current={
              selected?.kind === 'group' && item.id === selected.id
                ? 'true'
                : undefined
            }
            onClick={() => onChoose({ kind: 'group', id: item.id })}
          >
            <span className="access__mark">
              <Users aria-hidden="true" />
            </span>
            <span className="access__name">{item.name}</span>
            <span className="faint small tabular">{item.members.length}</span>
          </button>
        ))}
        <NewGroup
          busy={action.busy}
          onCreate={(name) =>
            action.run(() => admin.createGroup(name), `${name} is ready.`)
          }
        />
      </div>
      <div className="access__group">
        <h2>People</h2>
        {people.map((item) => (
          <button
            key={item.id}
            type="button"
            className="access__pick"
            aria-current={
              selected?.kind === 'user' && item.id === selected.id
                ? 'true'
                : undefined
            }
            onClick={() => onChoose({ kind: 'user', id: item.id })}
          >
            <PersonAvatar account={item.id} name={item.login} />
            <span className="access__name">{item.login}</span>
            {item.administrator && (
              <Shield className="access__admin" aria-label="Administrator" />
            )}
          </button>
        ))}
      </div>
    </nav>
  )
}

function firstChoice(groups: Group[], people: Account[]): Chosen | null {
  if (groups[0]) return { kind: 'group', id: groups[0].id }
  return people[0] ? { kind: 'user', id: people[0].id } : null
}

function NewGroup({
  busy,
  onCreate,
}: {
  busy: boolean
  onCreate: (name: string) => Promise<unknown>
}) {
  const [name, setName] = useState('')
  return (
    <form
      className="access__new"
      onSubmit={(event) => {
        event.preventDefault()
        const trimmed = name.trim()
        if (trimmed) void onCreate(trimmed).then(() => setName(''))
      }}
    >
      <Plus aria-hidden="true" />
      <input
        aria-label="New group name"
        placeholder="New group"
        maxLength={128}
        value={name}
        onChange={(event) => setName(event.target.value)}
      />
      {name.trim() && (
        <button className="btn btn--sm" disabled={busy}>
          Add
        </button>
      )}
    </form>
  )
}

type Action = ReturnType<typeof useAdminAction>

function GroupDetail({
  group,
  people,
  grants,
  action,
}: {
  group: Group
  people: Account[]
  grants: Grant[]
  action: Action
}) {
  const inGroup = people.filter((person) => group.members.includes(person.id))
  const others = people.filter((person) => !group.members.includes(person.id))
  const change = (account: string, present: boolean, done: string) =>
    void action.run(
      () => admin.membership(group.id, account, group.revision, present),
      done,
    )
  return (
    <>
      <header className="access__head">
        <h2>{group.name}</h2>
        <button
          type="button"
          className="btn btn--ghost btn--sm"
          disabled={action.busy}
          onClick={async () => {
            const yes = await confirmAction({
              title: `Delete ${group.name}?`,
              body: 'Its members lose what the group gave them.',
              action: 'Delete',
            })
            if (yes)
              void action.run(
                () => admin.deleteGroup(group.id, group.revision),
                `${group.name} is gone.`,
              )
          }}
        >
          Delete group
        </button>
      </header>
      <section className="access__section">
        <h3>Members</h3>
        <ul className="access__members">
          {inGroup.map((person) => (
            <li key={person.id}>
              <PersonAvatar account={person.id} name={person.login} />
              {person.login}
              <button
                type="button"
                className="access__remove"
                aria-label={`Remove ${person.login} from ${group.name}`}
                disabled={action.busy}
                onClick={() =>
                  change(
                    person.id,
                    false,
                    `${person.login} left ${group.name}.`,
                  )
                }
              >
                <X />
              </button>
            </li>
          ))}
          {others.length > 0 && (
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
                  if (person)
                    change(
                      person.id,
                      true,
                      `${person.login} joined ${group.name}.`,
                    )
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
        {inGroup.length === 0 && (
          <p className="small faint">
            Nobody yet. What you allow below reaches everyone you add.
          </p>
        )}
      </section>
      <AccessTable
        recipient={{ kind: 'group', id: group.id }}
        own={grants.filter((grant) => grant.group_id === group.id)}
        inherited={() => null}
        action={action}
      />
    </>
  )
}

function PersonDetail({
  person,
  groups,
  grants,
  action,
}: {
  person: Account
  groups: Group[]
  grants: Grant[]
  action: Action
}) {
  const theirGroups = groups.filter((group) =>
    group.members.includes(person.id),
  )
  return (
    <>
      <header className="access__head">
        <h2>{person.login}</h2>
        {theirGroups.length > 0 && (
          <span className="small faint">
            In {theirGroups.map((group) => group.name).join(', ')}
          </span>
        )}
      </header>
      {person.administrator ? (
        <p className="access__note">
          <Shield aria-hidden="true" />
          Administrators can do everything, on every agent and room. Nothing
          here needs granting.
        </p>
      ) : (
        <AccessTable
          recipient={{ kind: 'user', id: person.id }}
          own={grants.filter((grant) => grant.account_id === person.id)}
          inherited={(target) => throughGroups(theirGroups, grants, target)}
          action={action}
        />
      )}
    </>
  )
}

function throughGroups(groups: Group[], grants: Grant[], target: Target) {
  for (const level of ['manage', 'chat', 'view', 'custom'] as const)
    for (const group of groups) {
      const given = grantsOn(
        grants.filter((grant) => grant.group_id === group.id),
        target,
      )
      if (levelOf(given, target) === level)
        return `${LABEL[level]} through ${group.name}`
    }
  return null
}

function AccessTable({
  recipient,
  own,
  inherited,
  action,
}: {
  recipient: { kind: 'user' | 'group'; id: string }
  own: Grant[]
  inherited: (target: Target) => string | null
  action: Action
}) {
  const agents = useAgents()
  const rooms = useRooms()
  const [search, setSearch] = useState('')
  const match = (name: string) =>
    name.toLowerCase().includes(search.trim().toLowerCase())
  const set = (target: Target, name: string, level: Level) => {
    if (level === 'custom') return
    void action.run(
      () => applyLevel(recipient, own, target, level),
      `${LABEL[level]} on ${name}.`,
    )
  }
  const row = (target: Target, name: string, icon: React.ReactNode) => {
    const level = levelOf(grantsOn(own, target), target)
    const through = inherited(target)
    return (
      <li key={`${target.kind}:${target.id}`} className="access__row">
        {icon}
        <span className="access__target">
          <span className="access__name">{name}</span>
          {through && <span className="small faint">{through}</span>}
        </span>
        <select
          className="input access__level"
          aria-label={`Access to ${name}`}
          data-level={level}
          value={level}
          disabled={action.busy}
          onChange={(event) => set(target, name, event.target.value as Level)}
        >
          {level === 'custom' && <option value="custom">Custom</option>}
          {LEVELS.map((item) => (
            <option key={item.id} value={item.id}>
              {item.id === 'none' && through ? 'Nothing extra' : item.label}
            </option>
          ))}
        </select>
      </li>
    )
  }
  const shownAgents = agents.data?.filter((agent) => match(agent.name)) ?? []
  const shownRooms = rooms.data?.filter((room) => match(room.name)) ?? []
  return (
    <section className="access__section">
      <div className="access__table-head">
        <h3>What they can reach</h3>
        <label className="access__search">
          <Search aria-hidden="true" />
          <input
            type="search"
            aria-label="Filter agents and rooms"
            placeholder="Filter"
            value={search}
            onChange={(event) => setSearch(event.target.value)}
          />
        </label>
      </div>
      <dl className="access__legend small">
        {(['view', 'chat', 'manage'] as const).map((level) => (
          <div key={level}>
            <dt>{LABEL[level]}</dt>
            <dd>{MEANING[level]}</dd>
          </div>
        ))}
      </dl>
      <ul className="access__rows">
        {shownAgents.map((agent: AgentSummary) =>
          row(
            { kind: 'agent', id: agent.id },
            agent.name,
            <Cube avatar={agent.avatar} size={24} />,
          ),
        )}
        {shownRooms.map((room: RoomSummary) =>
          row(
            { kind: 'room', id: room.id },
            room.name,
            <span className="access__mark">
              <Hash aria-hidden="true" />
            </span>,
          ),
        )}
      </ul>
      {shownAgents.length + shownRooms.length === 0 && (
        <p className="small faint">Nothing matches that.</p>
      )}
    </section>
  )
}
