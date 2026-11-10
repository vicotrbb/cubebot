import { useQueryClient } from '@tanstack/react-query'
import { createFileRoute, useNavigate } from '@tanstack/react-router'
import { useState } from 'react'
import { api, failure, ok } from '../api/client'
import { keys, useAgents, useSession } from '../app/queries'
import { AddMember, MemberList } from '../components/MemberPicker'
import {
  agentOptions,
  memberKey,
  without,
  type MemberOption,
} from '../components/members'
import { Alert, Field } from '../components/primitives'
import { settlePeople, usePeopleCandidates } from '../features/rooms/Members'

export const Route = createFileRoute('/_app/rooms/new')({ component: NewRoom })

function NewRoom() {
  const agents = useAgents()
  const administrator = Boolean(useSession().data?.administrator)
  const { people, grants } = usePeopleCandidates(administrator)
  const navigate = useNavigate()
  const client = useQueryClient()
  const [name, setName] = useState('')
  const [members, setMembers] = useState<MemberOption[]>([])
  const ids = (kind: MemberOption['kind']) =>
    members.filter((member) => member.kind === kind).map((member) => member.id)
  const [requestId] = useState(() => crypto.randomUUID())
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  return (
    <div className="page">
      <header className="page__head">
        <h1>New room</h1>
      </header>
      <div className="page__body page__body--narrow">
        <form
          className="form"
          onSubmit={async (event) => {
            event.preventDefault()
            setBusy(true)
            setError('')
            try {
              const room = await ok(
                api.POST('/api/rooms', {
                  body: { request_id: requestId, name, agents: ids('agent') },
                }),
              )
              await settlePeople(room.id, [], ids('person'), grants)
              await client.invalidateQueries({ queryKey: keys.rooms })
              await navigate({
                to: '/rooms/$roomId',
                params: { roomId: room.id },
              })
            } catch (error) {
              setError(failure(error))
            } finally {
              setBusy(false)
            }
          }}
        >
          <Field label="Name">
            <input
              value={name}
              onChange={(event) => setName(event.target.value)}
              maxLength={128}
              required
              autoFocus
            />
          </Field>
          <fieldset className="member-field">
            <legend>Members</legend>
            <p className="small faint">
              Optional. Agents in a room can delegate to and message each other;
              people here can read and write in it. Add anyone later from the
              room.
            </p>
            <MemberList
              members={members}
              busy={busy}
              removable={() => true}
              onRemove={(gone) =>
                setMembers((current) =>
                  current.filter(
                    (member) => memberKey(member) !== memberKey(gone),
                  ),
                )
              }
              empty="No members yet."
            />
            <AddMember
              options={without(
                [...agentOptions(agents.data ?? []), ...people],
                members,
              )}
              busy={busy}
              placeholder={
                administrator ? 'Search agents and people' : 'Search agents'
              }
              onAdd={(member) => setMembers((current) => [...current, member])}
            />
          </fieldset>
          <Alert>{error}</Alert>
          <div className="actions">
            <button
              className="btn btn--primary"
              disabled={busy || !name.trim()}
            >
              {busy ? 'Creating…' : 'Create room'}
            </button>
            <button
              type="button"
              className="btn btn--ghost"
              onClick={() => void navigate({ to: '/' })}
            >
              Cancel
            </button>
          </div>
        </form>
      </div>
    </div>
  )
}
