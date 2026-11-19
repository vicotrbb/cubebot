import { useQuery, useQueryClient } from '@tanstack/react-query'
import { Link } from '@tanstack/react-router'
import { AtSign } from 'lucide-react'
import { useState } from 'react'
import { admin } from '../../api/admin'
import { api, failure, ok, type Room } from '../../api/client'
import type { AgentSummary } from '../../api/types'
import { keys } from '../../app/queries'
import { CardSelect, type CardOption } from '../../components/CardSelect'
import { Cube } from '../../components/Cube'
import { AddMember, MemberList } from '../../components/MemberPicker'
import {
  agentOptions,
  personOptions,
  without,
  type MemberOption,
} from '../../components/members'
import { Alert } from '../../components/primitives'
import type { Grant } from '../platform/access'
import { applyLevel } from '../platform/applyLevel'

export function usePeopleCandidates(enabled: boolean) {
  const accounts = useQuery({
    queryKey: ['accounts'],
    queryFn: admin.accounts,
    enabled,
  })
  const grants = useQuery({
    queryKey: ['grants'],
    queryFn: admin.grants,
    enabled,
  })
  return {
    people: personOptions(
      (accounts.data ?? []).filter((person) => !person.administrator),
    ),
    grants: grants.data ?? [],
  }
}

export async function settlePeople(
  room: string,
  before: string[],
  after: string[],
  grants: Grant[],
) {
  const own = (person: string) =>
    grants.filter((grant) => grant.account_id === person)
  const target = { kind: 'room', id: room } as const
  for (const person of after.filter((id) => !before.includes(id)))
    await applyLevel({ kind: 'user', id: person }, own(person), target, 'chat')
  for (const person of before.filter((id) => !after.includes(id)))
    await applyLevel({ kind: 'user', id: person }, own(person), target, 'none')
}

export function roomMembers(room: Room, agents: AgentSummary[]) {
  const known = new Map(agents.map((agent) => [agent.id, agent]))
  return [
    ...room.members.map((member): MemberOption => ({
      kind: 'agent',
      id: member.id,
      name: member.name,
      agent: known.get(member.id),
    })),
    ...room.people.map((person): MemberOption => ({
      kind: 'person',
      id: person.id,
      name: person.name,
    })),
  ]
}

export function Members({
  room,
  agents,
  administrator,
  onSaved,
}: {
  room: Room
  agents: AgentSummary[]
  administrator: boolean
  onSaved: (room: Room) => void
}) {
  const client = useQueryClient()
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  const agentsEditable = room.manage && !room.team && !room.archived
  const peopleEditable = administrator && !room.archived
  const { people, grants } = usePeopleCandidates(peopleEditable)
  const members = roomMembers(room, agents)
  const options = without(
    [
      ...(agentsEditable ? agentOptions(agents) : []),
      ...(peopleEditable ? people : []),
    ],
    members,
  )
  const run = async (action: () => Promise<void>) => {
    setBusy(true)
    setError('')
    try {
      await action()
    } catch (caught) {
      setError(failure(caught))
    } finally {
      setBusy(false)
    }
  }
  const setAgents = (ids: string[], lead = room.lead ?? null) =>
    run(async () =>
      onSaved(
        await ok(
          api.PUT('/api/rooms/{room}/members', {
            params: { path: { room: room.id } },
            body: {
              agents: ids,
              lead: lead && ids.includes(lead) ? lead : null,
            },
          }),
        ),
      ),
    )
  const setPeople = (after: string[]) =>
    run(async () => {
      await settlePeople(
        room.id,
        room.people.map((person) => person.id),
        after,
        grants,
      )
      await Promise.all([
        client.invalidateQueries({ queryKey: keys.rooms }),
        client.invalidateQueries({ queryKey: ['grants'] }),
      ])
    })
  const agentIds = room.members.map((member) => member.id)
  const personIds = room.people.map((person) => person.id)
  const add = (member: MemberOption) =>
    member.kind === 'agent'
      ? setAgents([...agentIds, member.id])
      : setPeople([...personIds, member.id])
  const remove = (member: MemberOption) =>
    member.kind === 'agent'
      ? setAgents(agentIds.filter((id) => id !== member.id))
      : setPeople(personIds.filter((id) => id !== member.id))
  return (
    <div className="room-members">
      {room.team && (
        <p className="small faint">
          Its agents follow the{' '}
          <Link to="/teams/$teamId" params={{ teamId: room.team }}>
            team
          </Link>
          : its members and the managers of the teams under it.
        </p>
      )}
      <MemberList
        members={members}
        busy={busy}
        removable={(member) =>
          member.kind === 'agent' ? agentsEditable : peopleEditable
        }
        onRemove={(member) => void remove(member)}
        empty="No one is in this room yet. Add an agent so messages get an answer."
      />
      {(agentsEditable || peopleEditable) && (
        <AddMember
          options={options}
          busy={busy}
          placeholder={
            agentsEditable && peopleEditable
              ? 'Search agents and people'
              : agentsEditable
                ? 'Search agents'
                : 'Search people'
          }
          onAdd={(member) => void add(member)}
        />
      )}
      <Lead
        room={room}
        agents={agents}
        editable={agentsEditable}
        onPick={(lead) => {
          if (!busy) void setAgents(agentIds, lead)
        }}
      />
      <Alert>{error}</Alert>
      <p className="small faint">
        Agents in a room can delegate to and message each other. People here can
        read and write in it; administrators reach every room.
      </p>
    </div>
  )
}

const FIRST = 'first'

function Lead({
  room,
  agents,
  editable,
  onPick,
}: {
  room: Room
  agents: AgentSummary[]
  editable: boolean
  onPick: (lead: string | null) => void
}) {
  const known = new Map(agents.map((agent) => [agent.id, agent]))
  const lead = room.members.find((member) => member.id === room.lead)
  if (room.members.length < 2) return null
  if (room.team || !editable)
    return (
      <p className="small faint">
        {lead
          ? `${lead.name} leads requests that mention several agents${room.team ? ', as the team’s manager' : ''}.`
          : 'The first agent mentioned leads requests that mention several agents.'}
      </p>
    )
  const options: CardOption<string>[] = [
    {
      value: FIRST,
      title: 'The first agent mentioned',
      detail: 'Whoever is named first plans the work and gives out the parts.',
      mark: <AtSign size={18} aria-hidden="true" />,
    },
    ...room.members.map((member) => ({
      value: member.id,
      title: member.name,
      detail: `Leads every request that mentions several agents here.`,
      mark: (
        <Cube
          agentId={member.id}
          avatar={known.get(member.id)?.avatar ?? null}
          mood="idle"
          size={24}
        />
      ),
    })),
  ]
  return (
    <div className="room-members__lead">
      <h3>Lead</h3>
      <p className="small faint">
        When a message mentions several agents, the lead plans the work, gives
        out the parts and writes the one answer.
      </p>
      <CardSelect
        label="Lead when several agents are mentioned"
        value={lead?.id ?? FIRST}
        onChange={(value) => onPick(value === FIRST ? null : value)}
        options={options}
      />
    </div>
  )
}
