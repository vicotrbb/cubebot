import { useQuery } from '@tanstack/react-query'
import { Link } from '@tanstack/react-router'
import { Shield, UserPlus, Users } from 'lucide-react'
import { useState } from 'react'
import { admin } from '../../api/admin'
import type { AgentSummary } from '../../api/types'
import { PersonAvatar } from '../../components/PersonAvatar'
import { Alert, Skeleton, Status } from '../../components/primitives'
import {
  grantsOn,
  LABEL,
  levelOf,
  LEVELS,
  MEANING,
  type Grant,
  type Level,
  type Target,
} from '../platform/access'
import { applyLevel, type Recipient } from '../platform/applyLevel'
import { useAdminAction } from '../platform/useAdminAction'
import '../platform/platform.css'

type Team = Awaited<ReturnType<typeof admin.teams>>[number]
type Account = Awaited<ReturnType<typeof admin.accounts>>[number]
type Holder = {
  recipient: Recipient
  name: string
  own: Grant[]
  level: Level
  team?: Team
}

const active = <T extends { active: boolean }>(list: T[] | undefined) =>
  list?.filter((item) => item.active) ?? []

/** Who can reach this agent and how far, set in the same levels as Platform › Teams and access. */
export function Access({ agent }: { agent: AgentSummary }) {
  const accounts = useQuery({ queryKey: ['accounts'], queryFn: admin.accounts })
  const teams = useQuery({ queryKey: ['teams'], queryFn: admin.teams })
  const grants = useQuery({ queryKey: ['grants'], queryFn: admin.grants })
  const action = useAdminAction(['grants', 'access-events'])
  const target: Target = { kind: 'agent', id: agent.id }
  if (accounts.isPending || teams.isPending || grants.isPending)
    return (
      <section className="panel__section">
        <Skeleton lines={5} />
      </section>
    )
  const people = active(accounts.data)
  const holders = holdersOf(
    active(teams.data),
    people.filter((person) => !person.administrator),
    grantsOn(grants.data ?? [], target),
    target,
  )
  const reaching = holders.filter(
    (holder) =>
      holder.level !== 'none' || throughTeams(holder, holders) !== null,
  )
  const admins = people.filter((person) => person.administrator)
  const set = (holder: Holder, level: Level) => {
    if (level === 'custom') return
    void action.run(
      () => applyLevel(holder.recipient, holder.own, target, level),
      level === 'none'
        ? `${holder.name} no longer reaches ${agent.name}.`
        : `${holder.name}: ${LABEL[level].toLowerCase()}.`,
    )
  }
  return (
    <section className="panel__section agent-access">
      <h2>Who can reach {agent.name}</h2>
      <dl className="access__legend small agent-access__legend">
        {(['view', 'chat', 'manage'] as const).map((level) => (
          <div key={level}>
            <dt>{LABEL[level]}</dt>
            <dd>{MEANING[level]}</dd>
          </div>
        ))}
      </dl>
      <Alert>{action.error}</Alert>
      <Status>{action.notice}</Status>
      <ul className="access__rows">
        {reaching.map((holder) => (
          <HolderRow
            key={`${holder.recipient.kind}:${holder.recipient.id}`}
            holder={holder}
            through={throughTeams(holder, holders)}
            busy={action.busy}
            onLevel={(level) => set(holder, level)}
          />
        ))}
        {admins.length > 0 && (
          <li className="access__row">
            <span className="access__mark">
              <Shield aria-hidden="true" />
            </span>
            <span className="access__target">
              <span className="access__name">
                {admins.map((person) => person.login).join(', ')}
              </span>
              <span className="small faint">
                {admins.length === 1 ? 'Administrator' : 'Administrators'}:
                everything, through their role
              </span>
            </span>
          </li>
        )}
      </ul>
      {reaching.length === 0 && (
        <p className="small faint">
          Nobody else reaches {agent.name} yet. Give someone access below.
        </p>
      )}
      <AddHolder
        holders={holders.filter((holder) => !reaching.includes(holder))}
        busy={action.busy}
        onAdd={set}
      />
      <p className="small faint">
        Every agent and room at once:{' '}
        <Link to="/platform/teams">Teams and access</Link>. Each change is in
        the <Link to="/platform/audit">audit log</Link>.
      </p>
    </section>
  )
}

function holdersOf(
  teams: Team[],
  people: Account[],
  onAgent: Grant[],
  target: Target,
): Holder[] {
  const holder = (recipient: Recipient, name: string, own: Grant[]) => ({
    recipient,
    name,
    own,
    level: levelOf(own, target),
  })
  return [
    ...teams.map((team) => ({
      ...holder(
        { kind: 'team', id: team.id },
        team.name,
        onAgent.filter((grant) => grant.team_id === team.id),
      ),
      team,
    })),
    ...people.map((person) =>
      holder(
        { kind: 'user', id: person.id },
        person.login,
        onAgent.filter((grant) => grant.account_id === person.id),
      ),
    ),
  ]
}

/** The best level a person also holds through a team they are in. */
function throughTeams(holder: Holder, holders: Holder[]) {
  if (holder.recipient.kind !== 'user') return null
  const id = holder.recipient.id
  const order: Level[] = ['manage', 'chat', 'view', 'custom']
  const via = holders
    .filter((other) => other.team?.members.includes(id))
    .filter((other) => other.level !== 'none')
    .sort((a, b) => order.indexOf(a.level) - order.indexOf(b.level))[0]
  return via ? `${LABEL[via.level]} through ${via.name}` : null
}

function HolderRow({
  holder,
  through,
  busy,
  onLevel,
}: {
  holder: Holder
  through: string | null
  busy: boolean
  onLevel: (level: Level) => void
}) {
  const team = holder.team
  const detail =
    holder.level === 'custom'
      ? holder.own
          .map((grant) => grant.capability.replaceAll('_', ' '))
          .join(', ')
      : team
        ? `Team of ${team.members.length}`
        : through
  return (
    <li className="access__row">
      {team ? (
        <span className="access__mark">
          <Users aria-hidden="true" />
        </span>
      ) : (
        <PersonAvatar
          account={holder.recipient.id}
          name={holder.name}
          className="agent-access__avatar"
        />
      )}
      <span className="access__target">
        <span className="access__name">{holder.name}</span>
        {detail && <span className="small faint">{detail}</span>}
      </span>
      <select
        className="input access__level"
        aria-label={`${holder.name}'s access`}
        data-level={holder.level}
        value={holder.level}
        disabled={busy}
        onChange={(event) => onLevel(event.target.value as Level)}
      >
        {holder.level === 'custom' && <option value="custom">Custom</option>}
        {LEVELS.map((item) => (
          <option key={item.id} value={item.id}>
            {item.id !== 'none'
              ? item.label
              : holder.level === 'none'
                ? 'Nothing extra'
                : 'Remove'}
          </option>
        ))}
      </select>
    </li>
  )
}

function AddHolder({
  holders,
  busy,
  onAdd,
}: {
  holders: Holder[]
  busy: boolean
  onAdd: (holder: Holder, level: Level) => void
}) {
  const [chosen, setChosen] = useState('')
  const [level, setLevel] = useState<Level>('chat')
  if (holders.length === 0) return null
  const key = (holder: Holder) =>
    `${holder.recipient.kind}:${holder.recipient.id}`
  const picked = holders.find((holder) => key(holder) === chosen)
  return (
    <form
      className="agent-access__add"
      onSubmit={(event) => {
        event.preventDefault()
        if (!picked) return
        onAdd(picked, level)
        setChosen('')
      }}
    >
      <h3>Give access</h3>
      <div className="agent-access__add-row">
        <select
          className="input"
          aria-label="Person or team"
          value={chosen}
          onChange={(event) => setChosen(event.target.value)}
        >
          <option value="" disabled>
            Person or team…
          </option>
          {holders.map((holder) => (
            <option key={key(holder)} value={key(holder)}>
              {holder.team ? `${holder.name} (team)` : holder.name}
            </option>
          ))}
        </select>
        <select
          className="input access__level"
          aria-label="Level"
          value={level}
          onChange={(event) => setLevel(event.target.value as Level)}
        >
          {LEVELS.filter((item) => item.id !== 'none').map((item) => (
            <option key={item.id} value={item.id}>
              {item.label}
            </option>
          ))}
        </select>
        <button className="btn btn--primary btn--sm" disabled={busy || !picked}>
          <UserPlus aria-hidden="true" />
          Add
        </button>
      </div>
    </form>
  )
}
