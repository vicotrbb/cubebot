import { useQuery, useQueryClient } from '@tanstack/react-query'
import { Link } from '@tanstack/react-router'
import { Eye, Shield, UserPlus, Users } from 'lucide-react'
import { useState } from 'react'
import { admin } from '../../api/admin'
import { failure } from '../../api/client'
import type { AgentSummary } from '../../api/types'
import { keys, useSession, useStanding } from '../../app/queries'
import { PersonAvatar } from '../../components/PersonAvatar'
import {
  Alert,
  confirmAction,
  Skeleton,
  Status,
} from '../../components/primitives'
import {
  grantsOn,
  LABEL,
  levelOf,
  levelsWithin,
  MEANING,
  type Grant,
  type Level,
  type Target,
} from '../platform/access'
import { applyLevel, type Recipient } from '../platform/applyLevel'
import { useAdminAction } from '../platform/useAdminAction'
import '../platform/platform.css'

type Person = Awaited<ReturnType<typeof admin.directory>>['people'][number]
type Group = { id: string; name: string; members?: string[] }
type Choice = ReturnType<typeof levelsWithin>[number]
type Holder = {
  recipient: Recipient
  name: string
  own: Grant[]
  level: Level
  group?: Group
}

const nameOf = (person: Person) => person.name || person.login
const active = <T extends { active: boolean }>(list: T[] | undefined) =>
  list?.filter((item) => item.active) ?? []

export function useResourceAccess(id: string, enabled = true) {
  return useQuery({
    queryKey: ['resource-access', id],
    queryFn: () => admin.resourceAccess(id),
    enabled,
  })
}

/** The agent's Access tab: anyone who manages it shares it, up to what they hold. */
export function Access({ agent }: { agent: AgentSummary }) {
  const administrator = Boolean(useSession().data?.administrator)
  return (
    <Sharing
      target={{ kind: 'agent', id: agent.id }}
      name={agent.name}
      held={agent.capabilities}
      editable={administrator || agent.capabilities.includes('agent_manage')}
    />
  )
}

/** Administrators see group membership; everyone else shares from the directory. */
function useSharingData(target: Target, administrator: boolean) {
  const directory = useQuery({
    queryKey: ['directory'],
    queryFn: admin.directory,
  })
  const groups = useQuery({
    queryKey: ['groups'],
    queryFn: admin.groups,
    enabled: administrator,
  })
  const access = useResourceAccess(target.id)
  const failed = directory.error ?? access.error
  return {
    pending:
      directory.isPending ||
      access.isPending ||
      (administrator && groups.isPending),
    error: failed ? failure(failed) : '',
    people: directory.data?.people ?? [],
    groups: administrator
      ? active(groups.data)
      : (directory.data?.groups ?? ([] as Group[])),
    owner: access.data?.owner ?? null,
    grants: grantsOn(access.data?.grants ?? [], target),
  }
}

/** Who can reach an agent or room and how far, in the same levels as Platform › Groups and access. */
export function Sharing({
  target,
  name,
  held,
  editable,
  transferable = true,
}: {
  target: Target
  name: string
  /** What the viewer holds on it; nobody shares more than that. */
  held: string[]
  editable: boolean
  transferable?: boolean
}) {
  const session = useSession().data
  const solo = useStanding().data?.solo
  const administrator = Boolean(session?.administrator)
  const data = useSharingData(target, administrator)
  if (solo)
    return (
      <section className="panel__section">
        <p className="small faint">
          You are the only person here, so there is nobody to share {name} with
          yet.
        </p>
      </section>
    )
  if (data.pending || data.error)
    return (
      <section className="panel__section">
        {data.error ? <Alert>{data.error}</Alert> : <Skeleton lines={5} />}
      </section>
    )
  return (
    <SharingBody
      target={target}
      name={name}
      data={data}
      me={session?.id ?? ''}
      administrator={administrator}
      choices={
        editable
          ? levelsWithin(target, administrator ? null : new Set(held))
          : []
      }
      transferable={transferable}
    />
  )
}

function SharingBody({
  target,
  name,
  data,
  me,
  administrator,
  choices,
  transferable,
}: {
  target: Target
  name: string
  data: ReturnType<typeof useSharingData>
  me: string
  administrator: boolean
  choices: Choice[]
  transferable: boolean
}) {
  const action = useAdminAction([
    'resource-access',
    'grants',
    'access-events',
    'rooms',
  ])
  const holders = holdersOf(
    data.groups,
    data.people,
    data.grants,
    target,
    data.owner,
  )
  const reaching = holders.filter(
    (holder) =>
      holder.level !== 'none' || throughGroups(holder, holders) !== null,
  )
  const owner = data.people.find((person) => person.id === data.owner)
  const set = (holder: Holder, level: Level) => {
    if (level === 'custom') return
    void action.run(
      () => applyLevel(holder.recipient, holder.own, target, level),
      level === 'none'
        ? `${holder.name} no longer reaches ${name}.`
        : `${holder.name}: ${LABEL[level].toLowerCase()}.`,
    )
  }
  return (
    <section className="panel__section agent-access">
      <h2>Who can reach {name}</h2>
      <Legend />
      <Alert>{action.error}</Alert>
      <Status>{action.notice}</Status>
      <ul className="access__rows">
        {owner && <OwnerRow owner={owner} me={me} />}
        {reaching.map((holder) => (
          <HolderRow
            key={`${holder.recipient.kind}:${holder.recipient.id}`}
            holder={holder}
            through={throughGroups(holder, holders)}
            choices={choices}
            busy={action.busy}
            onLevel={(level) => set(holder, level)}
          />
        ))}
        <Overseers people={data.people} owner={data.owner} />
      </ul>
      {reaching.length === 0 && (
        <p className="small faint">
          Nobody else reaches {name} yet.
          {choices.length > 1 && ' Give someone access below.'}
        </p>
      )}
      <AddHolder
        holders={holders.filter((holder) => !reaching.includes(holder))}
        choices={choices.filter((choice) => choice.id !== 'none')}
        busy={action.busy}
        onAdd={set}
      />
      {transferable && (administrator || me === data.owner) && (
        <TransferOwnership
          target={target}
          name={name}
          owner={data.owner}
          me={me}
          people={data.people}
        />
      )}
      {administrator && (
        <p className="small faint">
          Every agent and room at once:{' '}
          <Link to="/platform/groups">Groups and access</Link>. Each change is
          in the <Link to="/platform/audit">audit log</Link>.
        </p>
      )}
    </section>
  )
}

function Legend() {
  return (
    <dl className="access__legend small agent-access__legend">
      {(['view', 'chat', 'manage'] as const).map((level) => (
        <div key={level}>
          <dt>{LABEL[level]}</dt>
          <dd>{MEANING[level]}</dd>
        </div>
      ))}
    </dl>
  )
}

/** Administrators and auditors reach everything through their role, so they are not shared with. */
function holdersOf(
  groups: Group[],
  people: Person[],
  onTarget: Grant[],
  target: Target,
  owner: string | null,
): Holder[] {
  const holder = (recipient: Recipient, name: string, own: Grant[]) => ({
    recipient,
    name,
    own,
    level: levelOf(own, target),
  })
  return [
    ...groups.map((group) => ({
      ...holder(
        { kind: 'group', id: group.id },
        group.name,
        onTarget.filter((grant) => grant.group_id === group.id),
      ),
      group,
    })),
    ...people
      .filter((person) => person.role === 'member' || person.role === 'guest')
      .filter((person) => person.id !== owner)
      .map((person) =>
        holder(
          { kind: 'user', id: person.id },
          nameOf(person),
          onTarget.filter((grant) => grant.account_id === person.id),
        ),
      ),
  ]
}

/** The best level a person also holds through a group they are in. */
function throughGroups(holder: Holder, holders: Holder[]) {
  if (holder.recipient.kind !== 'user') return null
  const id = holder.recipient.id
  const order: Level[] = ['manage', 'chat', 'view', 'custom']
  const via = holders
    .filter((other) => other.group?.members?.includes(id))
    .filter((other) => other.level !== 'none')
    .sort((a, b) => order.indexOf(a.level) - order.indexOf(b.level))[0]
  return via ? `${LABEL[via.level]} through ${via.name}` : null
}

function OwnerRow({ owner, me }: { owner: Person; me: string }) {
  return (
    <li className="access__row">
      <PersonAvatar
        account={owner.id}
        name={nameOf(owner)}
        className="agent-access__avatar"
      />
      <span className="access__target">
        <span className="access__name">
          {nameOf(owner)}
          {owner.id === me && ' (you)'}
        </span>
        <span className="small faint">Can do everything, and share it</span>
      </span>
      <span className="access__level small">Owner</span>
    </li>
  )
}

function Overseers({
  people,
  owner,
}: {
  people: Person[]
  owner: string | null
}) {
  const others = people.filter((person) => person.id !== owner)
  const admins = others.filter((person) => person.role === 'administrator')
  const auditors = others.filter((person) => person.role === 'auditor')
  return (
    <>
      {admins.length > 0 && (
        <RoleRow
          people={admins}
          icon={<Shield aria-hidden="true" />}
          what={`${admins.length === 1 ? 'Administrator' : 'Administrators'}: everything, through their role`}
        />
      )}
      {auditors.length > 0 && (
        <RoleRow
          people={auditors}
          icon={<Eye aria-hidden="true" />}
          what={`${auditors.length === 1 ? 'Auditor' : 'Auditors'}: read everything, through their role`}
        />
      )}
    </>
  )
}

function RoleRow({
  people,
  icon,
  what,
}: {
  people: Person[]
  icon: React.ReactNode
  what: string
}) {
  return (
    <li className="access__row">
      <span className="access__mark">{icon}</span>
      <span className="access__target">
        <span className="access__name">{people.map(nameOf).join(', ')}</span>
        <span className="small faint">{what}</span>
      </span>
    </li>
  )
}

function HolderRow({
  holder,
  through,
  choices,
  busy,
  onLevel,
}: {
  holder: Holder
  through: string | null
  choices: Choice[]
  busy: boolean
  onLevel: (level: Level) => void
}) {
  const group = holder.group
  const detail =
    holder.level === 'custom'
      ? holder.own
          .map((grant) => grant.capability.replaceAll('_', ' '))
          .join(', ')
      : group
        ? group.members
          ? `Group of ${group.members.length}`
          : 'Group'
        : through
  return (
    <li className="access__row">
      {group ? (
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
      {choices.length > 0 ? (
        <LevelSelect
          holder={holder}
          choices={choices}
          busy={busy}
          onLevel={onLevel}
        />
      ) : (
        <span className="access__level small">{LABEL[holder.level]}</span>
      )}
    </li>
  )
}

/** The current level stays listed even when it is more than the viewer could hand out. */
function LevelSelect({
  holder,
  choices,
  busy,
  onLevel,
}: {
  holder: Holder
  choices: Choice[]
  busy: boolean
  onLevel: (level: Level) => void
}) {
  const offered = choices.some((choice) => choice.id === holder.level)
  return (
    <select
      className="input access__level"
      aria-label={`${holder.name}'s access`}
      data-level={holder.level}
      value={holder.level}
      disabled={busy}
      onChange={(event) => onLevel(event.target.value as Level)}
    >
      {!offered && (
        <option value={holder.level} disabled>
          {LABEL[holder.level]}
        </option>
      )}
      {choices.map((item) => (
        <option key={item.id} value={item.id}>
          {item.id !== 'none'
            ? item.label
            : holder.level === 'none'
              ? 'Nothing extra'
              : 'Remove'}
        </option>
      ))}
    </select>
  )
}

function AddHolder({
  holders,
  choices,
  busy,
  onAdd,
}: {
  holders: Holder[]
  choices: Choice[]
  busy: boolean
  onAdd: (holder: Holder, level: Level) => void
}) {
  const [chosen, setChosen] = useState('')
  const [pickedLevel, setLevel] = useState<Level | ''>('')
  if (holders.length === 0 || choices.length === 0) return null
  const level =
    pickedLevel ||
    (choices.find((choice) => choice.id === 'chat') ?? choices[0])?.id ||
    'view'
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
          aria-label="Person or group"
          value={chosen}
          onChange={(event) => setChosen(event.target.value)}
        >
          <option value="" disabled>
            Person or group…
          </option>
          {holders.map((holder) => (
            <option key={key(holder)} value={key(holder)}>
              {holder.group ? `${holder.name} (group)` : holder.name}
            </option>
          ))}
        </select>
        <select
          className="input access__level"
          aria-label="Level"
          value={level}
          onChange={(event) => setLevel(event.target.value as Level)}
        >
          {choices.map((item) => (
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

/** The new owner must be an active member or administrator; the server checks again. */
function TransferOwnership({
  target,
  name,
  owner,
  me,
  people,
}: {
  target: Target
  name: string
  owner: string | null
  me: string
  people: Person[]
}) {
  const client = useQueryClient()
  const action = useAdminAction(['resource-access', 'rooms', 'access-events'])
  const [to, setTo] = useState('')
  const candidates = people.filter(
    (person) =>
      person.id !== owner &&
      (person.role === 'member' || person.role === 'administrator'),
  )
  if (candidates.length === 0) return null
  const chosen = candidates.find((person) => person.id === to)
  const transfer = async () => {
    if (!chosen) return
    const yes = await confirmAction({
      title: `Transfer ${name} to ${nameOf(chosen)}?`,
      body:
        owner === me
          ? `${nameOf(chosen)} becomes its owner and can change, share or hand it on. You keep only the access shared with you or given by your role.`
          : `${nameOf(chosen)} becomes its owner and can change, share or hand it on.`,
      action: 'Transfer ownership',
    })
    if (!yes) return
    await action.run(
      async () => {
        await admin.transfer(target.kind, target.id, chosen.id)
        await Promise.all([
          client.invalidateQueries({ queryKey: keys.agents }),
          client.invalidateQueries({ queryKey: keys.standing }),
        ])
      },
      `${nameOf(chosen)} now owns ${name}.`,
    )
    setTo('')
  }
  return (
    <form
      className="agent-access__add"
      onSubmit={(event) => {
        event.preventDefault()
        void transfer()
      }}
    >
      <h3>Transfer ownership</h3>
      <Alert>{action.error}</Alert>
      <Status>{action.notice}</Status>
      <div className="agent-access__add-row">
        <select
          className="input"
          aria-label="New owner"
          value={to}
          onChange={(event) => setTo(event.target.value)}
        >
          <option value="" disabled>
            New owner…
          </option>
          {candidates.map((person) => (
            <option key={person.id} value={person.id}>
              {nameOf(person)}
            </option>
          ))}
        </select>
        <button className="btn btn--sm" disabled={action.busy || !chosen}>
          Transfer
        </button>
      </div>
    </form>
  )
}
