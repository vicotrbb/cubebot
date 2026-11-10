import { useQueryClient } from '@tanstack/react-query'
import { Link, useNavigate } from '@tanstack/react-router'
import { Hash, Pencil, Trash2 } from 'lucide-react'
import { useState } from 'react'
import { api, failure, ok } from '../../api/client'
import type { AgentSummary } from '../../api/types'
import {
  keys,
  notRetired,
  useAgents,
  useRooms,
  useSession,
  useTeams,
} from '../../app/queries'
import { MemberList } from '../../components/MemberPicker'
import type { MemberOption } from '../../components/members'
import { Cube, moodOf } from '../../components/Cube'
import {
  Alert,
  confirmAction,
  Empty,
  Skeleton,
} from '../../components/primitives'
import { managerOf, toSave, type Team } from './model'
import { OrgChart } from './OrgChart'
import { TeamEditor } from './TeamEditor'
import './teams.css'

export function TeamPage({ teamId }: { teamId: string }) {
  const teams = useTeams()
  const agents = useAgents()
  const rooms = useRooms()
  const session = useSession()
  const [editing, setEditing] = useState(false)
  const team = teams.data?.find((item) => item.id === teamId)
  if (teams.isPending || agents.isPending)
    return (
      <div className="page__body">
        <Skeleton lines={3} width="45%" />
      </div>
    )
  if (!team)
    return (
      <Empty title="This team is unavailable.">
        It was deleted, or its room is not shared with you.
      </Empty>
    )
  const all = teams.data ?? []
  const visible = notRetired(agents.data)
  const people = (
    rooms.data?.find((room) => room.id === team.room)?.people ?? []
  ).map((person): MemberOption => ({
    kind: 'person',
    id: person.id,
    name: person.name,
  }))
  const administrator = Boolean(session.data?.administrator)
  return (
    <div className="page team-page">
      <header className="page__head">
        <h1>{editing ? `Edit ${team.name}` : team.name}</h1>
        {!editing && (
          <div className="actions">
            <Link
              to="/rooms/$roomId"
              params={{ roomId: team.room }}
              className="btn btn--sm"
            >
              <Hash /> Team room
            </Link>
            {administrator && (
              <>
                <button
                  type="button"
                  className="btn btn--sm"
                  onClick={() => setEditing(true)}
                >
                  <Pencil /> Edit
                </button>
                <DeleteTeam team={team} teams={all} />
              </>
            )}
          </div>
        )}
      </header>
      <div className="page__body">
        {editing ? (
          <TeamEditor
            team={team}
            draft={toSave(team)}
            teams={all}
            agents={visible}
            people={people}
            onDone={() => setEditing(false)}
            onCancel={() => setEditing(false)}
          />
        ) : (
          <TeamDetails
            team={team}
            teams={all}
            agents={visible}
            people={people}
          />
        )}
      </div>
    </div>
  )
}

function TeamDetails({
  team,
  teams,
  agents,
  people,
}: {
  team: Team
  teams: Team[]
  agents: AgentSummary[]
  people: MemberOption[]
}) {
  const parent = teams.find((other) => other.id === team.parent)
  const boss = parent?.members.find((m) => m.agent === team.reports_to)
  const manager = managerOf(team)
  const named = new Map(agents.map((agent) => [agent.id, agent]))
  return (
    <>
      <div>
        {team.description && (
          <p className="team-page__lede">{team.description}</p>
        )}
        <p className="team-page__where">
          {manager && (
            <span>
              Managed by{' '}
              {team.members.find((m) => m.agent === manager.agent)?.name}
            </span>
          )}
          {parent && (
            <span>
              Under{' '}
              <Link to="/teams/$teamId" params={{ teamId: parent.id }}>
                {parent.name}
              </Link>
              {boss && `, reporting to ${boss.name}`}
            </span>
          )}
          <span>
            {team.members.length}{' '}
            {team.members.length === 1 ? 'agent' : 'agents'}
          </span>
        </p>
      </div>
      <section aria-labelledby="team-chart">
        <h2 id="team-chart">Org chart</h2>
        <div className="team-page__chart" tabIndex={0} aria-label="Org chart">
          <OrgChart teams={teams} agents={agents} root={team.id} />
        </div>
      </section>
      <section aria-labelledby="team-instructions">
        <h2 id="team-instructions">Instructions</h2>
        {team.instructions ? (
          <p className="team-page__text">{team.instructions}</p>
        ) : (
          <p className="faint small">
            No team instructions. Members follow only their own and their role.
          </p>
        )}
      </section>
      <section aria-labelledby="team-roles">
        <h2 id="team-roles">Roles</h2>
        {team.members.length === 0 && (
          <p className="faint small">
            No agents yet. Edit the team to add its manager and members.
          </p>
        )}
        <ul className="roles">
          {team.members.map((member) => {
            const agent = named.get(member.agent)
            const reportsTo = team.members.find(
              (m) => m.agent === member.reports_to,
            )
            return (
              <li key={member.agent}>
                <Cube
                  avatar={agent?.avatar ?? null}
                  mood={agent ? moodOf(agent) : 'idle'}
                  size={28}
                />
                <div className="roles__head">
                  <strong>{member.name}</strong>
                  <span>
                    {member.title}
                    {reportsTo ? `, reports to ${reportsTo.name}` : ', manager'}
                  </span>
                </div>
                {member.role ? (
                  <p>{member.role}</p>
                ) : (
                  <p className="faint">No role instructions.</p>
                )}
              </li>
            )
          })}
        </ul>
      </section>
      {people.length > 0 && (
        <section aria-labelledby="team-people">
          <h2 id="team-people">People</h2>
          <MemberList
            members={people}
            busy={false}
            removable={() => false}
            onRemove={() => undefined}
            empty=""
          />
        </section>
      )}
    </>
  )
}

function DeleteTeam({ team, teams }: { team: Team; teams: Team[] }) {
  const client = useQueryClient()
  const navigate = useNavigate()
  const [error, setError] = useState('')
  const under = teams.filter((other) => other.parent === team.id)
  return (
    <>
      <button
        type="button"
        className="btn btn--ghost btn--sm"
        disabled={under.length > 0}
        title={
          under.length > 0
            ? `Move ${under.map((t) => t.name).join(', ')} first`
            : undefined
        }
        onClick={async () => {
          const yes = await confirmAction({
            title: `Delete ${team.name}?`,
            body: 'Its members leave the team and its room is archived. The room’s history stays for administrators; tasks keep their record of the team.',
            action: 'Delete team',
          })
          if (!yes) return
          try {
            await ok(
              api.DELETE('/api/teams/{team}', {
                params: {
                  path: { team: team.id },
                  query: { expected_revision: team.revision },
                },
              }),
            )
            await Promise.all([
              client.invalidateQueries({ queryKey: keys.teams }),
              client.invalidateQueries({ queryKey: keys.rooms }),
            ])
            await navigate({ to: '/', search: { view: 'org' } })
          } catch (caught) {
            setError(failure(caught))
          }
        }}
      >
        <Trash2 /> Delete
      </button>
      {error && <Alert>{error}</Alert>}
    </>
  )
}
