import { useQueryClient } from '@tanstack/react-query'
import { Crown, Plus, Trash2 } from 'lucide-react'
import { useState } from 'react'
import { api, failure, ok } from '../../api/client'
import type { AgentSummary } from '../../api/types'
import { keys } from '../../app/queries'
import { Cube, moodOf } from '../../components/Cube'
import { Alert, Field } from '../../components/primitives'
import {
  bossChoices,
  makeManager,
  managerOf,
  problems,
  removeMember,
  teamsBelow,
  type SaveTeam,
  type Team,
} from './model'
import './teams.css'

const EMPTY: SaveTeam = {
  name: '',
  description: '',
  instructions: '',
  parent: null,
  reports_to: null,
  members: [],
}

/** Creates a team, or replaces `team` whole; the server checks the same rules again. */
export function TeamEditor({
  team,
  draft: initial,
  teams,
  agents,
  onDone,
  onCancel,
}: {
  team?: Team
  draft?: SaveTeam
  teams: Team[]
  agents: AgentSummary[]
  onDone: (team: Team) => void
  onCancel: () => void
}) {
  const client = useQueryClient()
  const [draft, setDraft] = useState<SaveTeam>(initial ?? EMPTY)
  const [requestId] = useState(() => crypto.randomUUID())
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  const [tried, setTried] = useState(false)
  const others = teams.filter((other) => other.id !== team?.id)
  const blocked = problems(draft, others)
  const set = (change: Partial<SaveTeam>) =>
    setDraft((current) => ({ ...current, ...change }))
  const save = async () => {
    setTried(true)
    if (blocked.length > 0) return
    setBusy(true)
    setError('')
    try {
      const saved = team
        ? await ok(
            api.PUT('/api/teams/{team}', {
              params: { path: { team: team.id } },
              body: { expected_revision: team.revision, team: draft },
            }),
          )
        : await ok(
            api.POST('/api/teams', {
              body: { request_id: requestId, team: draft },
            }),
          )
      await Promise.all([
        client.invalidateQueries({ queryKey: keys.teams }),
        client.invalidateQueries({ queryKey: keys.rooms }),
      ])
      onDone(saved)
    } catch (caught) {
      setError(failure(caught))
    } finally {
      setBusy(false)
    }
  }
  return (
    <form
      className="team-form"
      noValidate
      onSubmit={(event) => {
        event.preventDefault()
        void save()
      }}
    >
      <fieldset>
        <legend>About the team</legend>
        <Field label="Name">
          <input
            value={draft.name}
            onChange={(event) => set({ name: event.target.value })}
            maxLength={128}
            required
            autoFocus={!team}
          />
        </Field>
        <Field
          label="Description"
          hint="What the team is for. Its members and the task orchestrator read it."
        >
          <textarea
            value={draft.description}
            onChange={(event) => set({ description: event.target.value })}
            maxLength={2048}
            rows={2}
          />
        </Field>
        <Field
          label="Instructions"
          hint="Every member follows these in all of its work, beside its own instructions."
        >
          <textarea
            value={draft.instructions}
            onChange={(event) => set({ instructions: event.target.value })}
            maxLength={8192}
            rows={6}
          />
        </Field>
      </fieldset>
      <Placement draft={draft} team={team} teams={teams} set={set} />
      <Seats draft={draft} agents={agents} setDraft={setDraft} />
      {tried && blocked.length > 0 && (
        <ul className="team-form__problems" role="alert">
          {blocked.map((problem) => (
            <li key={problem}>{problem}</li>
          ))}
        </ul>
      )}
      <Alert>{error}</Alert>
      <div className="team-form__actions">
        <button className="btn btn--primary" disabled={busy}>
          {busy ? 'Saving…' : team ? 'Save team' : 'Create team'}
        </button>
        <button type="button" className="btn btn--ghost" onClick={onCancel}>
          Cancel
        </button>
      </div>
    </form>
  )
}

/** Where the team sits: under no team, or under a member of a team outside its own subtree. */
function Placement({
  draft,
  team,
  teams,
  set,
}: {
  draft: SaveTeam
  team?: Team | undefined
  teams: Team[]
  set: (change: Partial<SaveTeam>) => void
}) {
  const excluded = team ? teamsBelow(teams, team.id) : new Set<string>()
  const parents = teams.filter((other) => !excluded.has(other.id))
  const parent = teams.find((other) => other.id === draft.parent)
  return (
    <fieldset>
      <legend>Place in the organization</legend>
      <div className="team-form__pair">
        <Field label="Sits under">
          <select
            value={draft.parent ?? ''}
            onChange={(event) => {
              const next = teams.find((t) => t.id === event.target.value)
              set({
                parent: next?.id ?? null,
                reports_to: next ? (managerOf(next)?.agent ?? null) : null,
              })
            }}
          >
            <option value="">No team, it is at the top</option>
            {parents.map((other) => (
              <option key={other.id} value={other.id}>
                {other.name}
              </option>
            ))}
          </select>
        </Field>
        {parent && (
          <Field
            label="Its manager reports to"
            hint="They share the room of the team above."
          >
            <select
              value={draft.reports_to ?? ''}
              onChange={(event) => set({ reports_to: event.target.value })}
            >
              {parent.members.map((member) => (
                <option key={member.agent} value={member.agent}>
                  {member.name}, {member.title}
                </option>
              ))}
            </select>
          </Field>
        )}
      </div>
    </fieldset>
  )
}

function Seats({
  draft,
  agents,
  setDraft,
}: {
  draft: SaveTeam
  agents: AgentSummary[]
  setDraft: (change: (current: SaveTeam) => SaveTeam) => void
}) {
  const [adding, setAdding] = useState('')
  const seated = new Set(draft.members.map((member) => member.agent))
  const free = agents.filter((agent) => !seated.has(agent.id))
  const named = new Map(agents.map((agent) => [agent.id, agent]))
  const manager = managerOf(draft)?.agent
  const update = (agent: string, change: Partial<SaveTeam['members'][0]>) =>
    setDraft((current) => ({
      ...current,
      members: current.members.map((member) =>
        member.agent === agent ? { ...member, ...change } : member,
      ),
    }))
  return (
    <fieldset>
      <legend>Members</legend>
      <ul className="seats">
        {draft.members.map((member) => {
          const agent = named.get(member.agent)
          const name = agent?.name ?? 'An agent not shared with you'
          const isManager = member.agent === manager
          return (
            <li key={member.agent} className="seat" data-manager={isManager}>
              <Cube
                avatar={agent?.avatar ?? null}
                mood={agent ? moodOf(agent) : 'idle'}
                size={28}
              />
              <span className="seat__agent">
                <strong>{name}</strong>
                {isManager && <span className="seat__manager">Manager</span>}
              </span>
              <label className="field seat__title">
                <span className="sr-only">Title of {name}</span>
                <input
                  value={member.title}
                  placeholder="Title, such as Tech lead"
                  maxLength={64}
                  onChange={(event) =>
                    update(member.agent, { title: event.target.value })
                  }
                />
              </label>
              <label className="field seat__boss">
                <span className="sr-only">{name} reports to</span>
                {isManager ? (
                  <span className="small faint">Leads the team</span>
                ) : (
                  <select
                    value={member.reports_to ?? ''}
                    onChange={(event) =>
                      update(member.agent, { reports_to: event.target.value })
                    }
                  >
                    {bossChoices(draft, member.agent).map((boss) => (
                      <option key={boss.agent} value={boss.agent}>
                        Reports to {named.get(boss.agent)?.name ?? 'an agent'}
                      </option>
                    ))}
                  </select>
                )}
              </label>
              <span className="seat__actions">
                {!isManager && (
                  <button
                    type="button"
                    className="btn btn--ghost btn--icon btn--sm"
                    aria-label={`Make ${name} the manager`}
                    title="Make manager"
                    onClick={() =>
                      setDraft((current) => makeManager(current, member.agent))
                    }
                  >
                    <Crown />
                  </button>
                )}
                <button
                  type="button"
                  className="btn btn--ghost btn--icon btn--sm"
                  aria-label={`Remove ${name}`}
                  title={
                    isManager && draft.members.length > 1
                      ? 'Make another member manager first'
                      : 'Remove'
                  }
                  disabled={isManager && draft.members.length > 1}
                  onClick={() =>
                    setDraft((current) =>
                      isManager
                        ? { ...current, members: [] }
                        : removeMember(current, member.agent),
                    )
                  }
                >
                  <Trash2 />
                </button>
              </span>
              <label className="field seat__role">
                <span className="sr-only">Role of {name}</span>
                <textarea
                  value={member.role}
                  placeholder={`What ${name} does in this team, as instructions to it`}
                  maxLength={4096}
                  rows={2}
                  onChange={(event) =>
                    update(member.agent, { role: event.target.value })
                  }
                />
              </label>
            </li>
          )
        })}
      </ul>
      {free.length > 0 && (
        <div className="team-form__add">
          <label className="field">
            <span className="sr-only">Agent to add</span>
            <select
              value={adding}
              onChange={(event) => setAdding(event.target.value)}
            >
              <option value="">Choose an agent…</option>
              {free.map((agent) => (
                <option key={agent.id} value={agent.id}>
                  {agent.name}
                </option>
              ))}
            </select>
          </label>
          <button
            type="button"
            className="btn btn--sm"
            disabled={!adding}
            onClick={() => {
              setDraft((current) => ({
                ...current,
                members: [
                  ...current.members,
                  {
                    agent: adding,
                    title: current.members.length === 0 ? 'Manager' : '',
                    role: '',
                    reports_to: managerOf(current)?.agent ?? null,
                  },
                ],
              }))
              setAdding('')
            }}
          >
            <Plus /> Add to team
          </button>
        </div>
      )}
    </fieldset>
  )
}
