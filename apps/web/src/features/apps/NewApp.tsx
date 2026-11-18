import { useNavigate } from '@tanstack/react-router'
import { Send } from 'lucide-react'
import { useState } from 'react'
import { failure } from '../../api/client'
import { can, type AgentSummary } from '../../api/types'
import { notRetired, useAgents, useSession } from '../../app/queries'
import { CardSelect } from '../../components/CardSelect'
import { Cube, moodOf } from '../../components/Cube'
import { Alert, Dialog, Field } from '../../components/primitives'
import { doingOf } from '../agent/doing'
import { sendToAgent } from './api'

export function NewApp({
  open,
  agentId,
  onClose,
}: {
  open: boolean
  agentId: string | null
  onClose: () => void
}) {
  return (
    <Dialog open={open} title="New app" onClose={onClose}>
      <Request agentId={agentId} onDone={onClose} />
    </Dialog>
  )
}

function useSenders() {
  const agents = useAgents()
  const session = useSession()
  const me = session.data
  return notRetired(agents.data).filter(
    (agent) => me && can(agent, 'conversation_send', me),
  )
}

function Request({
  agentId,
  onDone,
}: {
  agentId: string | null
  onDone: () => void
}) {
  const senders = useSenders()
  const navigate = useNavigate()
  const [text, setText] = useState('')
  const [chosen, setChosen] = useState(agentId)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  const agent =
    senders.find((item) => item.id === chosen) ?? senders.at(0) ?? null
  if (!agent)
    return (
      <p className="muted">
        You need an agent you can message before you can ask for an app.
      </p>
    )
  const submit = async () => {
    setBusy(true)
    setError('')
    try {
      await sendToAgent(agent.id, `Please build a Cubebot app: ${text.trim()}`)
      onDone()
      await navigate({ to: '/agents/$agentId', params: { agentId: agent.id } })
    } catch (caught) {
      setError(failure(caught))
      setBusy(false)
    }
  }
  return (
    <form
      className="form new-app"
      onSubmit={(event) => {
        event.preventDefault()
        void submit()
      }}
    >
      <Field label="What should it do?">
        <textarea
          required
          maxLength={4000}
          autoFocus
          rows={4}
          value={text}
          placeholder="A dashboard of my recovery, sleep and strain for the last 30 days"
          onChange={(event) => setText(event.target.value)}
        />
      </Field>
      <div className="field">
        <span>Agent</span>
        <CardSelect
          label="Agent"
          value={agent.id}
          onChange={setChosen}
          options={senders.map(option)}
        />
      </div>
      <p className="faint small">
        This goes to {agent.name}&apos;s chat as your request, and it builds the
        app there.
      </p>
      <Alert>{error}</Alert>
      <div className="actions dialog__actions">
        <button
          type="submit"
          className="btn btn--primary"
          disabled={busy || !text.trim()}
        >
          <Send aria-hidden="true" /> {busy ? 'Sending…' : 'Send'}
        </button>
      </div>
    </form>
  )
}

function option(agent: AgentSummary) {
  return {
    value: agent.id,
    title: agent.name,
    detail: agent.title || doingOf(agent),
    mark: <Cube avatar={agent.avatar} mood={moodOf(agent)} size={30} />,
  }
}
