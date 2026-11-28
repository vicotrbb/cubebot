import { useQueryClient } from '@tanstack/react-query'
import { Copy, Download, ExternalLink, ShieldCheck } from 'lucide-react'
import { type FormEvent, type ReactNode, useState } from 'react'
import { account } from '../../api/account'
import { ApiError, failure } from '../../api/client'
import {
  keys,
  useInstallation,
  useSession,
  useStanding,
} from '../../app/queries'
import {
  Alert,
  CopyButton,
  copyText,
  Field,
  Status,
} from '../../components/primitives'
import { CodeField, codeOf } from './CodeField'
import {
  groupedSecret,
  PASSWORD_MIN,
  passwordProblem,
  recoveryFile,
} from './model'
import { SESSIONS_KEY } from './Sessions'

/** Password and two-factor sign-in. */
export function Security() {
  return (
    <section className="profile__about" aria-labelledby="security">
      <div>
        <h3 id="security">Sign-in and security</h3>
        <p className="faint small">
          How you prove it is you when you sign in with a password.
        </p>
      </div>
      <Password />
      <TwoFactor />
    </section>
  )
}

function Password() {
  const [open, setOpen] = useState(false)
  const [notice, setNotice] = useState('')
  return (
    <div className="you__part">
      <div className="you__row">
        <div className="you__text">
          <h4>Password</h4>
          <p className="faint small">
            Changing it signs out every other browser and ends your API tokens.
          </p>
        </div>
        {!open && (
          <button
            type="button"
            className="btn btn--sm"
            onClick={() => {
              setNotice('')
              setOpen(true)
            }}
          >
            Change password
          </button>
        )}
      </div>
      <Status>{notice}</Status>
      {open && (
        <PasswordForm
          onCancel={() => setOpen(false)}
          onDone={() => {
            setOpen(false)
            setNotice(
              'Password changed. Every other session and token was ended.',
            )
          }}
        />
      )}
    </div>
  )
}

function PasswordForm({
  onCancel,
  onDone,
}: {
  onCancel: () => void
  onDone: () => void
}) {
  const client = useQueryClient()
  const session = useSession()
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  async function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    const form = new FormData(event.currentTarget)
    const next = String(form.get('new'))
    const problem = passwordProblem(next, String(form.get('confirm')))
    if (problem) return setError(problem)
    setBusy(true)
    setError('')
    try {
      await account.changePassword(String(form.get('current')), next)
      await client.invalidateQueries({ queryKey: SESSIONS_KEY })
      onDone()
    } catch (caught) {
      setError(
        caught instanceof ApiError && caught.status === 400
          ? 'Your current password is not right. Enter it again.'
          : failure(caught),
      )
    } finally {
      setBusy(false)
    }
  }
  return (
    <form className="form you__form" onSubmit={(e) => void submit(e)}>
      {/* Lets password managers file the new password under the right login. */}
      <input
        type="text"
        name="username"
        autoComplete="username"
        value={session.data?.login ?? ''}
        readOnly
        hidden
      />
      <Field label="Current password">
        <input
          name="current"
          type="password"
          autoComplete="current-password"
          required
          autoFocus
        />
      </Field>
      <div className="profile__pair">
        <Field
          label="New password"
          hint={`At least ${PASSWORD_MIN} characters`}
        >
          <input
            name="new"
            type="password"
            autoComplete="new-password"
            minLength={PASSWORD_MIN}
            required
          />
        </Field>
        <Field label="Confirm new password">
          <input
            name="confirm"
            type="password"
            autoComplete="new-password"
            required
          />
        </Field>
      </div>
      <Alert>{error}</Alert>
      <div className="actions">
        <button className="btn btn--primary btn--sm" disabled={busy}>
          {busy ? 'Changing…' : 'Change password'}
        </button>
        <button
          type="button"
          className="btn btn--ghost btn--sm"
          onClick={onCancel}
        >
          Cancel
        </button>
      </div>
    </form>
  )
}

function TwoFactor() {
  const standing = useStanding()
  const client = useQueryClient()
  const [codes, setCodes] = useState<string[] | null>(null)
  const [notice, setNotice] = useState('')
  const refresh = () => client.invalidateQueries({ queryKey: keys.standing })
  const on = standing.data?.two_factor === true
  return (
    <div className="you__part">
      <Status>{notice}</Status>
      {codes ? (
        <RecoveryCodes codes={codes} onDone={() => setCodes(null)} />
      ) : on ? (
        <TurnOff
          onDone={async () => {
            await refresh()
            setNotice('Two-factor is off. Sign-in asks only for your password.')
          }}
        />
      ) : (
        standing.data && (
          <TurnOn
            onAlreadyOn={refresh}
            onCodes={async (fresh) => {
              setNotice('')
              setCodes(fresh)
              await refresh()
            }}
          />
        )
      )}
    </div>
  )
}

function TwoFactorHead({
  on,
  children,
}: {
  on: boolean
  children?: ReactNode
}) {
  return (
    <div className="you__row">
      <div className="you__text">
        <h4>
          Two-factor sign-in{' '}
          <span className={on ? 'chip chip--sage' : 'chip'}>
            {on ? 'On' : 'Off'}
          </span>
        </h4>
        <p className="faint small">
          {on
            ? 'Signing in with your password also asks for a code from your authenticator app.'
            : 'Ask for a six-digit code from an authenticator app, such as 1Password or Google Authenticator, after your password.'}
        </p>
      </div>
      {children}
    </div>
  )
}

type Enrollment = { secret: string; uri: string }

function TurnOn({
  onAlreadyOn,
  onCodes,
}: {
  onAlreadyOn: () => Promise<unknown>
  onCodes: (codes: string[]) => Promise<unknown>
}) {
  const [enrollment, setEnrollment] = useState<Enrollment | null>(null)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  async function start() {
    setBusy(true)
    setError('')
    try {
      setEnrollment(await account.enrollTwoFactor())
    } catch (caught) {
      if (caught instanceof ApiError && caught.status === 409)
        await onAlreadyOn()
      else setError(enrollProblem(caught))
    } finally {
      setBusy(false)
    }
  }
  return (
    <>
      <TwoFactorHead on={false}>
        {!enrollment && (
          <button
            type="button"
            className="btn btn--sm"
            disabled={busy}
            onClick={() => void start()}
          >
            <ShieldCheck /> {busy ? 'Starting…' : 'Turn on'}
          </button>
        )}
      </TwoFactorHead>
      <Alert>{error}</Alert>
      {enrollment && (
        <Enroll
          enrollment={enrollment}
          onCancel={() => setEnrollment(null)}
          onCodes={onCodes}
        />
      )}
    </>
  )
}

function enrollProblem(caught: unknown) {
  return caught instanceof ApiError && caught.status === 503
    ? 'Two-factor needs the installation’s credential key, and this installation has none. Ask an administrator to set one.'
    : failure(caught)
}

function Enroll({
  enrollment,
  onCancel,
  onCodes,
}: {
  enrollment: Enrollment
  onCancel: () => void
  onCodes: (codes: string[]) => Promise<unknown>
}) {
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  async function confirm(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    setBusy(true)
    setError('')
    try {
      const { codes } = await account.confirmTwoFactor(
        codeOf(new FormData(event.currentTarget)),
      )
      await onCodes(codes)
    } catch (caught) {
      setError(
        caught instanceof ApiError && caught.status === 400
          ? 'That code did not match. Codes change every 30 seconds; enter the one showing now.'
          : failure(caught),
      )
      setBusy(false)
    }
  }
  return (
    <ol className="you__steps">
      <li>
        <p>
          In your authenticator app, add an account and enter this setup key. On
          a phone with the app, you can open the link instead.
        </p>
        <div className="you__secret">
          <code>{groupedSecret(enrollment.secret)}</code>
          <CopyButton text={enrollment.secret} label="Copy the setup key" />
        </div>
        <a className="link-btn small" href={enrollment.uri}>
          <ExternalLink aria-hidden="true" /> Open in your authenticator app
        </a>
      </li>
      <li>
        <form className="form you__form" onSubmit={(e) => void confirm(e)}>
          <Field label="Six-digit code it shows">
            <input
              name="code"
              required
              inputMode="numeric"
              autoComplete="one-time-code"
              pattern="[0-9 ]{6,7}"
              maxLength={7}
              placeholder="123456"
              disabled={busy}
            />
          </Field>
          <Alert>{error}</Alert>
          <div className="actions">
            <button className="btn btn--primary btn--sm" disabled={busy}>
              {busy ? 'Checking…' : 'Turn on two-factor'}
            </button>
            <button
              type="button"
              className="btn btn--ghost btn--sm"
              onClick={onCancel}
            >
              Cancel
            </button>
          </div>
        </form>
      </li>
    </ol>
  )
}

function RecoveryCodes({
  codes,
  onDone,
}: {
  codes: string[]
  onDone: () => void
}) {
  const session = useSession()
  const installation = useInstallation()
  const [copied, setCopied] = useState(false)
  const text = recoveryFile(
    codes,
    session.data?.login ?? 'you',
    installation.data?.name ?? 'Cubebot',
  )
  const download = () => {
    const link = document.createElement('a')
    link.href = URL.createObjectURL(new Blob([text], { type: 'text/plain' }))
    link.download = 'cubebot-recovery-codes.txt'
    link.click()
    URL.revokeObjectURL(link.href)
  }
  return (
    <div className="you__reveal" role="region" aria-labelledby="recovery">
      <h4 id="recovery">Two-factor is on. Save your recovery codes.</h4>
      <p className="small">
        Each code signs you in once if you lose your authenticator app. They are
        not shown again, so keep them somewhere safe outside Cubebot.
      </p>
      <ul className="you__codes" aria-label="Recovery codes">
        {codes.map((code) => (
          <li key={code}>
            <code>{code}</code>
          </li>
        ))}
      </ul>
      <div className="actions">
        <button
          type="button"
          className="btn btn--sm"
          onClick={() =>
            void copyText(codes.join('\n')).then(() => setCopied(true))
          }
        >
          <Copy /> {copied ? 'Copied' : 'Copy codes'}
        </button>
        <button type="button" className="btn btn--sm" onClick={download}>
          <Download /> Download as .txt
        </button>
        <button
          type="button"
          className="btn btn--primary btn--sm"
          onClick={onDone}
        >
          I saved them
        </button>
      </div>
    </div>
  )
}

function TurnOff({ onDone }: { onDone: () => Promise<unknown> }) {
  const [open, setOpen] = useState(false)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  async function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    setBusy(true)
    setError('')
    try {
      await account.disableTwoFactor(codeOf(new FormData(event.currentTarget)))
      await onDone()
    } catch (caught) {
      setError(
        caught instanceof ApiError && caught.status === 400
          ? 'That code did not work. Enter the one showing in your authenticator app, or a recovery code you have not used.'
          : failure(caught),
      )
      setBusy(false)
    }
  }
  return (
    <>
      <TwoFactorHead on>
        {!open && (
          <button
            type="button"
            className="btn btn--ghost btn--sm"
            onClick={() => setOpen(true)}
          >
            Turn off
          </button>
        )}
      </TwoFactorHead>
      {open && (
        <form className="form you__form" onSubmit={(e) => void submit(e)}>
          <CodeField autoFocus disabled={busy} />
          <Alert>{error}</Alert>
          <div className="actions">
            <button className="btn btn--danger btn--sm" disabled={busy}>
              {busy ? 'Turning off…' : 'Turn off two-factor'}
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
      )}
    </>
  )
}
