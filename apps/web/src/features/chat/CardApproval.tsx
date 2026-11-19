import { CircleCheck, CircleX, CreditCard, Loader } from 'lucide-react'
import { useState } from 'react'
import { api, ApiError, type Turn } from '../../api/client'
import type { AgentSummary } from '../../api/types'
import { Alert } from '../../components/primitives'

type Tool = Turn['terminal'][number]
type State = 'pending' | 'settling' | 'approved' | 'declined'

export const isCardAsk = (tool: Tool) =>
  tool.name === 'vault_fill' && !!tool.request && !!tool.card

function refusal(status: number) {
  if (status === 403)
    return "Only someone who may control this agent's desktop can settle this; an installation card needs an administrator."
  if (status === 404)
    return 'Someone already settled this payment. Reload to see the outcome.'
  return new ApiError(status).message
}

function until(approved: string | null | undefined) {
  if (!approved) return ''
  return new Date(approved).toLocaleTimeString(undefined, {
    timeStyle: 'short',
  })
}

export function CardApproval({
  agent,
  tool,
  canSettle,
}: {
  agent: AgentSummary
  tool: Tool
  canSettle: boolean
}) {
  const card = tool.card
  const [state, setState] = useState<State>(
    (tool.approval as State | null) ?? 'pending',
  )
  const [declining, setDeclining] = useState(false)
  const [reason, setReason] = useState(tool.approval_note ?? '')
  const [error, setError] = useState('')
  if (!card || !tool.request) return null
  const request = tool.request
  async function settle(decline: boolean) {
    setState('settling')
    setError('')
    const path = { params: { path: { id: agent.id, request } } }
    const { response } = decline
      ? await api.POST('/api/agents/{id}/card-fills/{request}/decline', {
          ...path,
          body: { reason: reason.trim() },
        })
      : await api.POST('/api/agents/{id}/card-fills/{request}/approve', path)
    if (response.ok) return setState(decline ? 'declined' : 'approved')
    setState('pending')
    setError(refusal(response.status))
  }
  const pending = state === 'pending'
  return (
    <div className="ask card-ask" data-state={state}>
      <div className="ask__row">
        <span className="work__icon">
          <CreditCard />
        </span>
        <p className="card-ask__what">
          <span className="work__verb">Pay</span>{' '}
          <strong className="tabular">{card.amount}</strong>{' '}
          <span className="work__verb">on</span> <strong>{card.domain}</strong>
        </p>
        {pending && canSettle ? (
          <span className="ask__actions">
            <button
              type="button"
              className="btn btn--ghost btn--sm"
              onClick={() => setDeclining(true)}
            >
              Decline
            </button>
            <button
              type="button"
              className="btn btn--primary btn--sm"
              onClick={() => void settle(false)}
            >
              Approve payment
            </button>
          </span>
        ) : (
          <StateMark state={state} card={card} />
        )}
      </div>
      <p className="card-ask__card">
        {agent.name} asks to type the card <strong>{card.name}</strong>
        {card.last4 && (
          <span className="tabular">
            {' '}
            ({card.brand} ending {card.last4})
          </span>
        )}
        . Approving lets it fill this card on {card.domain} for the next 15
        minutes. The amount is what {agent.name} read on the page; check it
        there before you approve.
      </p>
      {error && (
        <div className="ask__body">
          <Alert>{error}</Alert>
        </div>
      )}
      {state === 'declined' && reason && (
        <p className="ask__note">Reason: {reason}</p>
      )}
      {pending && declining && canSettle && (
        <form
          className="creation-request__decline card-ask__decline"
          onSubmit={(event) => {
            event.preventDefault()
            void settle(true)
          }}
        >
          <input
            className="input"
            aria-label="Reason for declining, sent to the agent"
            value={reason}
            maxLength={2048}
            autoFocus
            placeholder={`Tell ${agent.name} why (optional)`}
            onChange={(event) => setReason(event.target.value)}
          />
          <button type="submit" className="btn btn--sm btn--danger">
            Decline
          </button>
          <button
            type="button"
            className="btn btn--ghost btn--sm"
            onClick={() => setDeclining(false)}
          >
            Cancel
          </button>
        </form>
      )}
    </div>
  )
}

function StateMark({
  state,
  card,
}: {
  state: State
  card: NonNullable<Tool['card']>
}) {
  if (state === 'settling')
    return (
      <span className="ask__state" data-state="approving">
        <Loader className="spin" aria-hidden="true" /> Working
      </span>
    )
  if (state === 'approved')
    return (
      <span className="ask__state" data-state="approved">
        <CircleCheck aria-hidden="true" /> Approved
        {card.approved_until && ` until ${until(card.approved_until)}`}
      </span>
    )
  if (state === 'declined')
    return (
      <span className="ask__state" data-state="declined">
        <CircleX aria-hidden="true" /> Declined
      </span>
    )
  return (
    <span className="ask__state" data-state="pending">
      Waiting for a person
    </span>
  )
}
