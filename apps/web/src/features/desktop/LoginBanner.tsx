import { KeyRound } from 'lucide-react'
import { useCallback, useEffect, useState } from 'react'
import { api, failure, ok, type LoginHandoff } from '../../api/client'
import type { AgentSummary } from '../../api/types'
import { Alert } from '../../components/primitives'
import { ItemForm } from '../vault/ItemForm'
import './desktop.css'

/** A pending login handoff (docs/adr/sandbox.md#live-browser-viewing-and-explicit-human-takeover): the agent waits for a person to log in for it. */
export function LoginBanner({
  agent,
  holding = false,
  canTake = true,
  takeLabel = 'Take control',
  onTake,
}: {
  agent: AgentSummary
  holding?: boolean
  /** False where control cannot be taken here, such as on a phone's desktop page. */
  canTake?: boolean
  takeLabel?: string
  onTake: () => void
}) {
  const [handoff, setHandoff] = useState<LoginHandoff | null>(null)
  const refresh = useCallback(async () => {
    const { data, response } = await api.GET('/api/agents/{id}/login', {
      params: { path: { id: agent.id } },
    })
    // Without browser.view the request is refused and no banner shows.
    if (response.ok) setHandoff(data ?? null)
  }, [agent.id])
  const pending = agent.pending_login
  useEffect(() => {
    // `pending_login` on the installation stream says when to ask; the site and reason
    // come from this route, polled every 3 s while the login is pending.
    if (!pending) return
    const poll = () => void refresh().catch(() => undefined)
    poll()
    const timer = setInterval(poll, 3000)
    return () => clearInterval(timer)
  }, [refresh, pending])
  if (!handoff || !pending) return null
  return (
    <section
      className={
        holding ? 'login-handoff login-handoff--holding' : 'login-handoff'
      }
      aria-label="Login request"
    >
      <span className="login-handoff__icon" aria-hidden="true">
        <KeyRound />
      </span>
      <div className="login-handoff__body">
        <p className="login-handoff__title">
          {holding
            ? `Log in to ${handoff.site}, then hand back control`
            : `${agent.name} needs you to log in to ${handoff.site}`}
        </p>
        {holding ? (
          <p className="login-handoff__reason">
            Type the password and any code on the desktop yourself; {agent.name}{' '}
            continues when you hand back. Save the login to the vault so it can
            sign in alone next time.
          </p>
        ) : (
          <p className="login-handoff__reason">{handoff.reason}</p>
        )}
      </div>
      {holding ? (
        <SaveLogin agent={agent} site={handoff.site} />
      ) : (
        <Respond
          agent={agent}
          canTake={canTake}
          takeLabel={takeLabel}
          onTake={onTake}
          onDeclined={() => setHandoff(null)}
        />
      )}
    </section>
  )
}

function Respond({
  agent,
  canTake,
  takeLabel,
  onTake,
  onDeclined,
}: {
  agent: AgentSummary
  canTake: boolean
  takeLabel: string
  onTake: () => void
  onDeclined: () => void
}) {
  const [declining, setDeclining] = useState(false)
  const [reason, setReason] = useState('')
  const [error, setError] = useState('')
  const [busy, setBusy] = useState(false)
  async function decline() {
    setBusy(true)
    setError('')
    try {
      await ok(
        api.POST('/api/agents/{id}/login/decline', {
          params: { path: { id: agent.id } },
          body: { reason },
        }),
      )
      onDeclined()
    } catch (error) {
      setError(failure(error))
    } finally {
      setBusy(false)
    }
  }
  if (declining)
    return (
      <form
        className="login-handoff__decline"
        onSubmit={(event) => {
          event.preventDefault()
          void decline()
        }}
      >
        <input
          className="input"
          aria-label="Reason for declining, sent to the agent"
          value={reason}
          maxLength={2048}
          // Focus follows the person's own click on Decline.
          autoFocus
          placeholder={`Tell ${agent.name} why (optional)`}
          onChange={(event) => setReason(event.target.value)}
        />
        <button
          type="submit"
          className="btn btn--sm btn--danger"
          disabled={busy}
        >
          {busy ? 'Declining…' : 'Decline'}
        </button>
        <button
          type="button"
          className="btn btn--ghost btn--sm"
          onClick={() => setDeclining(false)}
        >
          Cancel
        </button>
        <Alert>{error}</Alert>
      </form>
    )
  return (
    <div className="login-handoff__actions">
      <button
        type="button"
        className="btn btn--ghost btn--sm"
        onClick={() => setDeclining(true)}
      >
        Decline
      </button>
      {canTake ? (
        <button
          type="button"
          className="btn btn--primary btn--sm"
          onClick={onTake}
        >
          {takeLabel}
        </button>
      ) : (
        <span className="faint small">Take control from a computer.</span>
      )}
    </div>
  )
}

function SaveLogin({ agent, site }: { agent: AgentSummary; site: string }) {
  const [saving, setSaving] = useState(false)
  if (!saving)
    return (
      <div className="login-handoff__actions">
        <button
          type="button"
          className="btn btn--sm"
          onClick={() => setSaving(true)}
        >
          Save login to vault
        </button>
      </div>
    )
  return (
    <div className="login-handoff__form">
      <ItemForm
        agent={agent}
        site={site}
        fill
        onDone={() => setSaving(false)}
      />
    </div>
  )
}
