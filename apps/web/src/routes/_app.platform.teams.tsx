import { useQuery } from '@tanstack/react-query'
import { createFileRoute } from '@tanstack/react-router'
import { Hash, Plus, Search, Shield, Users, X } from 'lucide-react'
import { useState } from 'react'
import { admin } from '../api/admin'
import type { AgentSummary, RoomSummary } from '../api/types'
import { useAgents, useRooms } from '../app/queries'
import { Cube } from '../components/Cube'
import { PersonAvatar } from '../components/PersonAvatar'
import { Alert, Skeleton, Status } from '../components/primitives'
import {
  changesTo,
  grantsOn,
  LABEL,
  levelOf,
  LEVELS,
  MEANING,
  type Grant,
  type Level,
  type Target,
} from '../features/platform/access'
import { useAdminAction } from '../features/platform/useAdminAction'
import '../features/platform/platform.css'

export const Route = createFileRoute('/_app/platform/teams')({
  component: TeamsAndAccess,
})

type Team = Awaited<ReturnType<typeof admin.teams>>[number]
type Account = Awaited<ReturnType<typeof admin.accounts>>[number]
type Chosen = { kind: 'team' | 'user'; id: string }

const active = <T extends { active: boolean }>(list: T[] | undefined) =>
  list?.filter((item) => item.active) ?? []

/** Pick a person or team, see what they can reach, and change it where you read it. */
function TeamsAndAccess() {
  const teams = useQuery({ queryKey: ['teams'], queryFn: admin.teams })
  const accounts = useQuery({ queryKey: ['accounts'], queryFn: admin.accounts })
  const grants = useQuery({ queryKey: ['grants'], queryFn: admin.grants })
  const action = useAdminAction(['teams', 'grants', 'access-events'])
  const [chosen, setChosen] = useState<Chosen | null>(null)
  const activeTeams = active(teams.data)
  const people = active(accounts.data)
  const selected = chosen ?? firstChoice(activeTeams, people)
  const team =
    selected?.kind === 'team'
      ? activeTeams.find((item) => item.id === selected.id)
      : undefined
  const person =
    selected?.kind === 'user'
      ? people.find((item) => item.id === selected.id)
      : undefined
  return (
    <div className="access">
      <Recipients
        teams={activeTeams}
        people={people}
        selected={selected}
        action={action}
        onChoose={setChosen}
      />
      <div className="access__detail">
        <Alert>{action.error}</Alert>
        <Status>{action.notice}</Status>
        {(teams.isPending || accounts.isPending) && <Skeleton lines={4} />}
        {team && (
          <TeamDetail
            team={team}
            people={people}
            grants={grants.data ?? []}
            action={action}
          />
        )}
        {person && (
          <PersonDetail
            person={person}
            teams={activeTeams}
            grants={grants.data ?? []}
            action={action}
          />
        )}
      </div>
    </div>
  )
}

/** Everyone access can be given to. */
function Recipients({
  teams,
  people,
  selected,
  action,
  onChoose,
}: {
  teams: Team[]
  people: Account[]
  selected: Chosen | null
  action: Action
  onChoose: (chosen: Chosen) => void
}) {
  return (
    <nav className="access__list" aria-label="People and teams">
      <div className="access__group">
        <h2>Teams</h2>
        {teams.map((item) => (
          <button
            key={item.id}
            type="button"
            className="access__pick"
            aria-current={
              selected?.kind === 'team' && item.id === selected.id
                ? 'true'
                : undefined
            }
            onClick={() => onChoose({ kind: 'team', id: item.id })}
          >
            <span className="access__mark">
              <Users aria-hidden="true" />
            </span>
            <span className="access__name">{item.name}</span>
            <span className="faint small tabular">{item.members.length}</span>
          </button>
        ))}
        <NewTeam
          busy={action.busy}
          onCreate={(name) =>
            action.run(() => admin.createTeam(name), `${name} is ready.`)
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

function firstChoice(teams: Team[], people: Account[]): Chosen | null {
  if (teams[0]) return { kind: 'team', id: teams[0].id }
  return people[0] ? { kind: 'user', id: people[0].id } : null
}

function NewTeam({
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
        aria-label="New team name"
        placeholder="New team"
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

function TeamDetail({
  team,
  people,
  grants,
  action,
}: {
  team: Team
  people: Account[]
  grants: Grant[]
  action: Action
}) {
  const inTeam = people.filter((person) => team.members.includes(person.id))
  const others = people.filter((person) => !team.members.includes(person.id))
  const change = (account: string, present: boolean, done: string) =>
    void action.run(
      () => admin.membership(team.id, account, team.revision, present),
      done,
    )
  return (
    <>
      <header className="access__head">
        <h2>{team.name}</h2>
        <button
          type="button"
          className="btn btn--ghost btn--sm"
          disabled={action.busy}
          onClick={() => {
            if (
              window.confirm(
                `Delete ${team.name}? Its members lose what the team gave them.`,
              )
            )
              void action.run(
                () => admin.deleteTeam(team.id, team.revision),
                `${team.name} is gone.`,
              )
          }}
        >
          Delete team
        </button>
      </header>
      <section className="access__section">
        <h3>Members</h3>
        <ul className="access__members">
          {inTeam.map((person) => (
            <li key={person.id}>
              <PersonAvatar account={person.id} name={person.login} />
              {person.login}
              <button
                type="button"
                className="access__remove"
                aria-label={`Remove ${person.login} from ${team.name}`}
                disabled={action.busy}
                onClick={() =>
                  change(person.id, false, `${person.login} left ${team.name}.`)
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
                aria-label={`Add someone to ${team.name}`}
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
                      `${person.login} joined ${team.name}.`,
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
        {inTeam.length === 0 && (
          <p className="small faint">
            Nobody yet. What you allow below reaches everyone you add.
          </p>
        )}
      </section>
      <AccessTable
        recipient={{ kind: 'team', id: team.id }}
        own={grants.filter((grant) => grant.team_id === team.id)}
        inherited={() => null}
        action={action}
      />
    </>
  )
}

function PersonDetail({
  person,
  teams,
  grants,
  action,
}: {
  person: Account
  teams: Team[]
  grants: Grant[]
  action: Action
}) {
  const theirTeams = teams.filter((team) => team.members.includes(person.id))
  return (
    <>
      <header className="access__head">
        <h2>{person.login}</h2>
        {theirTeams.length > 0 && (
          <span className="small faint">
            In {theirTeams.map((team) => team.name).join(', ')}
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
          inherited={(target) => throughTeams(theirTeams, grants, target)}
          action={action}
        />
      )}
    </>
  )
}

/** The strongest level a person's teams give them here, and which team gives it. */
function throughTeams(teams: Team[], grants: Grant[], target: Target) {
  for (const level of ['manage', 'chat', 'view', 'custom'] as const)
    for (const team of teams) {
      const given = grantsOn(
        grants.filter((grant) => grant.team_id === team.id),
        target,
      )
      if (levelOf(given, target) === level)
        return `${LABEL[level]} through ${team.name}`
    }
  return null
}

/** One row per agent and room, each with the level this recipient holds. */
function AccessTable({
  recipient,
  own,
  inherited,
  action,
}: {
  recipient: { kind: 'user' | 'team'; id: string }
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
    const { grant, revoke } = changesTo(grantsOn(own, target), target, level)
    void action.run(async () => {
      for (const item of revoke) await admin.revokeGrant(item.id, item.revision)
      for (const need of grant)
        await admin.grant(
          recipient,
          { kind: need.scope, id: target.id } as Parameters<
            typeof admin.grant
          >[1],
          need.capability,
        )
    }, `${LABEL[level]} on ${name}.`)
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
