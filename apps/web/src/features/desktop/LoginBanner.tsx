import { KeyRound } from 'lucide-react'
import { useCallback, useEffect, useState } from 'react'
import { api, failure, ok, type LoginHandoff } from '../../api/client'
import type { AgentSummary } from '../../api/types'
import { Alert } from '../../components/primitives'
import { ItemForm } from '../vault/ItemForm'
import './desktop.css'

/** A pending login handoff (ADR 0039): the agent waits for a person to log in for it. */
export function LoginBanner({
  agent,
  holding = false,
  onTake,
}: {
  agent: AgentSummary
  holding?: boolean
  onTake: () => void
}) {
  const [handoff, setHandoff] = useState<LoginHandoff | null>(null)
  const [reason, setReason] = useState('')
  const [error, setError] = useState('')
  const [busy, setBusy] = useState(false)
  const [saving, setSaving] = useState(false)
  const refresh = useCallback(async () => {
    const { data, response } = await api.GET('/api/agents/{id}/login', {
      params: { path: { id: agent.id } },
    })
    // Without browser.view the request is refused and no banner shows.
    if (response.ok) setHandoff(data ?? null)
  }, [agent.id])
  useEffect(() => {
    // shortcut: each open view polls every 3 s; `pending_login` on the installation stream
    // says when, but the site and reason still come from this route.
    const poll = () => void refresh().catch(() => undefined)
    poll()
    const timer = setInterval(poll, 3000)
    return () => clearInterval(timer)
  }, [refresh])
  if (!handoff) return null
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
      setHandoff(null)
      setReason('')
    } catch (error) {
      setError(failure(error))
    } finally {
      setBusy(false)
    }
  }
  return (
    <section className="login-handoff" aria-label="Login request">
      <KeyRound className="login-handoff__icon" />
      <div className="login-handoff__body">
        <p>
          <strong>
            {agent.name} asks you to log in on {handoff.site}.
          </strong>{' '}
          {handoff.reason}
        </p>
        {holding ? (
          <>
            <p className="small muted">
              Log in on the desktop, then hand back control to let the agent
              continue. To let the agent log in itself next time, save the login
              to its vault; Save and fill types the password where the cursor
              is.
            </p>
            {saving ? (
              <ItemForm
                agent={agent}
                site={handoff.site}
                fill
                onDone={() => setSaving(false)}
              />
            ) : (
              <div className="actions">
                <button
                  type="button"
                  className="btn btn--sm"
                  onClick={() => setSaving(true)}
                >
                  Save login
                </button>
              </div>
            )}
          </>
        ) : (
          <div className="actions">
            <button
              type="button"
              className="btn btn--primary btn--sm"
              onClick={onTake}
            >
              Take control
            </button>
            <input
              className="input"
              aria-label="Reason for declining"
              value={reason}
              maxLength={2048}
              placeholder="Reason for declining (optional, sent to the agent)"
              onChange={(event) => setReason(event.target.value)}
              style={{ flex: 1, minWidth: 200 }}
            />
            <button
              type="button"
              className="btn btn--sm"
              disabled={busy}
              onClick={decline}
            >
              {busy ? 'Declining…' : 'Decline'}
            </button>
          </div>
        )}
        <Alert>{error}</Alert>
      </div>
    </section>
  )
}
