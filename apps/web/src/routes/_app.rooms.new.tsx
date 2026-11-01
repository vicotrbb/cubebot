import { useQueryClient } from '@tanstack/react-query'
import { createFileRoute, useNavigate } from '@tanstack/react-router'
import { useState } from 'react'
import { api, failure, ok } from '../api/client'
import { keys, useAgents } from '../app/queries'
import { Alert, Field } from '../components/primitives'
import { AgentChoices } from '../features/rooms/Members'

export const Route = createFileRoute('/_app/rooms/new')({ component: NewRoom })

function NewRoom() {
  const agents = useAgents()
  const navigate = useNavigate()
  const client = useQueryClient()
  const [name, setName] = useState('')
  const [members, setMembers] = useState<string[]>([])
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
                  body: { request_id: requestId, name, agents: members },
                }),
              )
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
          <AgentChoices
            agents={agents.data ?? []}
            chosen={members}
            onChange={setMembers}
          />
          <Alert>{error}</Alert>
          <div className="actions">
            <button
              className="btn btn--primary"
              disabled={busy || !name.trim() || members.length === 0}
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
