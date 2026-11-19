import { BellRing } from 'lucide-react'
import { useState } from 'react'
import { api, failure, ok } from '../../api/client'
import { usePush, type PushState } from '../../app/push'
import { Alert, Switch } from '../../components/primitives'

const WHAT =
  'A notification reaches this device when an agent asks you something, waits for an approval or a login, or its work stops.'

const EXPLAINED: Partial<Record<PushState, string>> = {
  install:
    'On iPhone and iPad, add Cubebot to your Home Screen first: tap Share, then Add to Home Screen, and open it from there.',
  denied:
    'Notifications are blocked for Cubebot in this browser. Allow them in its site settings, then come back here.',
  unsupported: 'This browser cannot show notifications.',
}

function TestButton() {
  const [result, setResult] = useState('')
  const [busy, setBusy] = useState(false)
  const send = async () => {
    setBusy(true)
    setResult('')
    try {
      const { delivered } = await ok(api.POST('/api/push/test'))
      setResult(
        delivered
          ? `Sent to ${delivered} ${delivered === 1 ? 'device' : 'devices'}.`
          : 'No device accepted it. Turn notifications off and on again.',
      )
    } catch (caught) {
      setResult(failure(caught))
    } finally {
      setBusy(false)
    }
  }
  return (
    <>
      <button
        type="button"
        className="btn btn--ghost btn--sm"
        disabled={busy}
        onClick={() => void send()}
      >
        <BellRing /> {busy ? 'Sending…' : 'Send a test'}
      </button>
      {result && (
        <p className="faint small profile__result" role="status">
          {result}
        </p>
      )}
    </>
  )
}

export function Notifications() {
  const push = usePush()
  const explained = EXPLAINED[push.state]
  const toggles = !explained
  return (
    <section className="profile__section" aria-labelledby="notifications">
      <div className="profile__text">
        <h3 id="notifications">Notifications</h3>
        <p className="faint small">{explained ?? WHAT}</p>
        {push.state === 'on' && <TestButton />}
        <Alert>{push.error}</Alert>
      </div>
      {toggles && (
        <Switch
          checked={push.state === 'on'}
          disabled={push.busy || push.state === 'checking'}
          label="Notifications on this device"
          onChange={(on) => void (on ? push.enable() : push.disable())}
        />
      )}
    </section>
  )
}
