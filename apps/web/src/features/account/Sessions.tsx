import { useQuery, useQueryClient } from '@tanstack/react-query'
import { KeyRound, Monitor, Plus } from 'lucide-react'
import { type FormEvent, useState } from 'react'
import { account } from '../../api/account'
import { failure } from '../../api/client'
import type { components } from '../../api/schema'
import { useStanding } from '../../app/queries'
import {
  Alert,
  confirmAction,
  CopyButton,
  Field,
  Skeleton,
  when,
} from '../../components/primitives'
import { fromUnix } from './model'

type Session = components['schemas']['SessionSummary']

export const SESSIONS_KEY = ['account-sessions']

const EXPIRIES: [number, string][] = [
  [7, '7 days'],
  [30, '30 days'],
  [90, '90 days'],
  [365, '1 year'],
]

/** Every browser signed in as you and every personal API token. */
export function Sessions() {
  const sessions = useQuery({
    queryKey: SESSIONS_KEY,
    queryFn: account.sessions,
  })
  const standing = useStanding()
  return (
    <section className="profile__about" aria-labelledby="sessions">
      <div>
        <h3 id="sessions">Sessions and tokens</h3>
        <p className="faint small">
          Browsers signed in as you and tokens scripts use to call the API as
          you. End anything you do not recognise.
        </p>
      </div>
      {sessions.isPending && <Skeleton lines={3} />}
      <Alert>{sessions.isError ? failure(sessions.error) : ''}</Alert>
      {sessions.data && (
        <ul className="entries" aria-label="Sessions and tokens">
          {sessions.data.map((session) => (
            <SessionRow key={session.id} session={session} />
          ))}
        </ul>
      )}
      {standing.data?.role === 'guest' ? (
        <p className="faint small">Guests cannot create API tokens.</p>
      ) : (
        <CreateToken />
      )}
    </section>
  )
}

function describe(session: Session) {
  const seen = when(fromUnix(session.last_seen_at_unix))
  const ends = new Date(session.expires_at_unix * 1000).toLocaleDateString(
    undefined,
    { month: 'short', day: 'numeric', year: 'numeric' },
  )
  if (session.name)
    return `Created ${when(fromUnix(session.created_at_unix))} · ${seen ? `last used ${seen}` : 'never used'} · expires ${ends}`
  return `Signed in ${when(fromUnix(session.created_at_unix))}${seen ? ` · last seen ${seen}` : ''} · ends ${ends}`
}

function SessionRow({ session }: { session: Session }) {
  const client = useQueryClient()
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  const token = Boolean(session.name)
  const label = session.name ?? 'Browser sign-in'
  async function end() {
    const yes = await confirmAction({
      title: token ? `Revoke ${label}?` : 'End this sign-in?',
      body: token
        ? 'Anything calling the API with this token stops working at once.'
        : 'That browser is signed out at once.',
      action: token ? 'Revoke token' : 'End session',
    })
    if (!yes) return
    setBusy(true)
    setError('')
    try {
      await account.endSession(session.id)
      await client.invalidateQueries({ queryKey: SESSIONS_KEY })
    } catch (caught) {
      setError(failure(caught))
      setBusy(false)
    }
  }
  const Icon = token ? KeyRound : Monitor
  return (
    <li className="entry">
      <span className="entry__icon" aria-hidden="true">
        <Icon />
      </span>
      <div className="entry__main">
        <span className="entry__name">
          {label}
          {session.current && (
            <span className="chip chip--sage you__current">This browser</span>
          )}
        </span>
        <span className="entry__detail">{describe(session)}</span>
        <Alert>{error}</Alert>
      </div>
      {!session.current && (
        <button
          type="button"
          className="btn btn--ghost btn--sm"
          disabled={busy}
          onClick={() => void end()}
        >
          {token ? 'Revoke' : 'End'}
        </button>
      )}
    </li>
  )
}

function CreateToken() {
  const client = useQueryClient()
  const [open, setOpen] = useState(false)
  const [token, setToken] = useState('')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  async function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    const form = new FormData(event.currentTarget)
    setBusy(true)
    setError('')
    try {
      const created = await account.createToken(
        String(form.get('name')).trim(),
        Number(form.get('days')),
      )
      setToken(created.token)
      setOpen(false)
      await client.invalidateQueries({ queryKey: SESSIONS_KEY })
    } catch (caught) {
      setError(failure(caught))
    } finally {
      setBusy(false)
    }
  }
  if (token) return <NewToken token={token} onDone={() => setToken('')} />
  if (!open)
    return (
      <div>
        <button
          type="button"
          className="btn btn--sm"
          onClick={() => setOpen(true)}
        >
          <Plus /> Create a token
        </button>
      </div>
    )
  return (
    <form className="form you__form" onSubmit={(e) => void submit(e)}>
      <div className="profile__pair">
        <Field label="Name" hint="What uses it">
          <input
            name="name"
            required
            maxLength={64}
            autoComplete="off"
            placeholder="Deploy script"
            autoFocus
          />
        </Field>
        <Field label="Expires after">
          <select name="days" defaultValue={30}>
            {EXPIRIES.map(([days, label]) => (
              <option key={days} value={days}>
                {label}
              </option>
            ))}
          </select>
        </Field>
      </div>
      <Alert>{error}</Alert>
      <div className="actions">
        <button className="btn btn--primary btn--sm" disabled={busy}>
          {busy ? 'Creating…' : 'Create token'}
        </button>
        <button
          type="button"
          className="btn btn--ghost btn--sm"
          onClick={() => setOpen(false)}
        >
          Cancel
        </button>
      </div>
    </form>
  )
}

function NewToken({ token, onDone }: { token: string; onDone: () => void }) {
  return (
    <div className="you__reveal" role="region" aria-labelledby="new-token">
      <h4 id="new-token">Copy your new token now. It is not shown again.</h4>
      <div className="you__secret">
        <code>{token}</code>
        <CopyButton text={token} label="Copy the token" />
      </div>
      <p className="small">
        It acts as you, with your access. Send it as{' '}
        <code>Authorization: Bearer &lt;token&gt;</code>. Revoke it here when
        you no longer need it.
      </p>
      <div className="actions">
        <button
          type="button"
          className="btn btn--primary btn--sm"
          onClick={onDone}
        >
          Done
        </button>
      </div>
    </div>
  )
}
