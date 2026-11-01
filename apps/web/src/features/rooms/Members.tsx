import { useState } from 'react'
import { api, failure, ok, type Room } from '../../api/client'
import type { AgentSummary } from '../../api/types'
import { Alert } from '../../components/primitives'

export function AgentChoices({
  agents,
  chosen,
  onChange,
}: {
  agents: Pick<AgentSummary, 'id' | 'name'>[]
  chosen: string[]
  onChange: (chosen: string[]) => void
}) {
  return (
    <fieldset
      className="form"
      style={{ border: 0, padding: 0, margin: 0, gap: 8 }}
    >
      <legend className="field" style={{ marginBottom: 6 }}>
        Agents
      </legend>
      <p className="small faint">
        Agents in a room can delegate to and message each other.
      </p>
      {agents.map((agent) => (
        <label key={agent.id} className="check">
          <input
            type="checkbox"
            checked={chosen.includes(agent.id)}
            onChange={(event) =>
              onChange(
                event.target.checked
                  ? [...chosen, agent.id]
                  : chosen.filter((id) => id !== agent.id),
              )
            }
          />
          {agent.name}
        </label>
      ))}
    </fieldset>
  )
}

/** Replaces the member set; removed agents keep the work already given to them. */
export function Members({
  room,
  agents,
  onSaved,
}: {
  room: Room
  agents: AgentSummary[]
  onSaved: (room: Room) => void
}) {
  const [chosen, setChosen] = useState(room.members.map((member) => member.id))
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  // Members this person cannot see as agents still show, so saving never drops them silently.
  const known = new Set(agents.map((agent) => agent.id))
  const listed = [
    ...agents,
    ...room.members.filter((member) => !known.has(member.id)),
  ]
  return (
    <form
      className="form"
      onSubmit={async (event) => {
        event.preventDefault()
        setBusy(true)
        setError('')
        try {
          onSaved(
            await ok(
              api.PUT('/api/rooms/{room}/members', {
                params: { path: { room: room.id } },
                body: { agents: chosen },
              }),
            ),
          )
        } catch (error) {
          setError(failure(error))
        } finally {
          setBusy(false)
        }
      }}
    >
      <AgentChoices agents={listed} chosen={chosen} onChange={setChosen} />
      <Alert>{error}</Alert>
      <div className="actions">
        <button
          className="btn btn--primary btn--sm"
          disabled={busy || chosen.length === 0}
        >
          {busy ? 'Saving…' : 'Save members'}
        </button>
      </div>
    </form>
  )
}
