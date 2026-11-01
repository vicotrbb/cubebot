import { useQueryClient } from '@tanstack/react-query'
import { createFileRoute, useNavigate } from '@tanstack/react-router'
import { useState } from 'react'
import { api, failure, ok } from '../api/client'
import { keys } from '../app/queries'
import { DEFAULT_AVATAR, type Avatar } from '../components/Cube'
import { Alert, Field } from '../components/primitives'
import { AvatarBuilder } from '../features/agent/AvatarBuilder'
import '../features/agent/about.css'

export const Route = createFileRoute('/_app/agents/new')({
  component: NewAgent,
})

function NewAgent() {
  const navigate = useNavigate()
  const client = useQueryClient()
  const [avatar, setAvatar] = useState<Avatar>(DEFAULT_AVATAR)
  const [requestId] = useState(() => crypto.randomUUID())
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  return (
    <div className="page">
      <header className="page__head">
        <h1>New agent</h1>
      </header>
      <div className="page__body page__body--narrow">
        <form
          className="form"
          onSubmit={async (event) => {
            event.preventDefault()
            setBusy(true)
            setError('')
            const data = new FormData(event.currentTarget)
            try {
              const made = await ok(
                api.POST('/api/agents', {
                  body: {
                    request_id: requestId,
                    name: String(data.get('name')),
                    instructions: String(data.get('instructions')),
                    avatar,
                  },
                }),
              )
              await client.invalidateQueries({ queryKey: keys.agents })
              await navigate({
                to: '/agents/$agentId',
                params: { agentId: made.id },
              })
            } catch (error) {
              setError(failure(error))
            } finally {
              setBusy(false)
            }
          }}
        >
          <AvatarBuilder value={avatar} onChange={setAvatar} />
          <Field label="Name">
            <input name="name" required maxLength={255} autoFocus />
          </Field>
          <Field
            label="Instructions"
            hint="The agent reads these at the start of every turn"
          >
            <textarea name="instructions" rows={8} maxLength={65536} />
          </Field>
          <p className="small faint">
            The agent starts asleep. Wake it from its Status tab once it has a
            provider; its conversation, memory and workspace persist from here
            on.
          </p>
          <Alert>{error}</Alert>
          <div className="actions">
            <button className="btn btn--primary" disabled={busy}>
              {busy ? 'Creating…' : 'Create agent'}
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
