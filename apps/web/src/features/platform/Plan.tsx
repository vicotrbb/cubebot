import { useQueryClient } from '@tanstack/react-query'
import { ExternalLink, LogIn, LogOut, RefreshCw } from 'lucide-react'
import { useEffect, useRef, useState } from 'react'
import { api, failure, ok } from '../../api/client'
import type { components } from '../../api/schema'
import {
  Alert,
  CopyButton,
  Field,
  confirmAction,
} from '../../components/primitives'
import { planProblem, signInProblem } from './subscription'

type Subscription = components['schemas']['SubscriptionSummary']
export type PlanKind = 'chatgpt' | 'supergrok'

export const PLANS: Record<
  PlanKind,
  { name: string; tiers: string; vendor: string }
> = {
  chatgpt: { name: 'ChatGPT', tiers: 'ChatGPT Plus or Pro', vendor: 'OpenAI' },
  supergrok: { name: 'Grok', tiers: 'SuperGrok or X Premium', vendor: 'xAI' },
}

/** The sign-in, or the signed-in plan, at the top of its provider's card. */
export function PlanBlock({
  plan,
  subscription,
  inUse,
  children,
  onChanged,
}: {
  plan: PlanKind
  subscription: Subscription | undefined
  inUse: boolean
  /** The model picker, shown only to the person whose plan it is. */
  children: React.ReactNode
  onChanged: (message: string) => Promise<unknown>
}) {
  const { name, tiers } = PLANS[plan]
  return (
    <section className="plan" aria-label={`${name} plan`}>
      <div className="plan__intro">
        <h3>Use your {name} plan</h3>
        <p className="small faint">
          {tiers} pays for agents instead of a key. A plan is personal: only
          agents created by the person who signed in use it.
        </p>
      </div>
      {subscription ? (
        <SignedIn
          plan={plan}
          subscription={subscription}
          inUse={inUse}
          onChanged={onChanged}
        >
          {children}
        </SignedIn>
      ) : plan === 'chatgpt' ? (
        <ChatgptSignIn onChanged={onChanged} />
      ) : (
        <GrokSignIn onChanged={onChanged} />
      )}
    </section>
  )
}

function SignedIn({
  plan,
  subscription,
  inUse,
  children,
  onChanged,
}: {
  plan: PlanKind
  subscription: Subscription
  inUse: boolean
  children: React.ReactNode
  onChanged: (message: string) => Promise<unknown>
}) {
  const { name } = PLANS[plan]
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  const problem = planProblem(plan, subscription)
  const signOut = async () => {
    const yes =
      !inUse ||
      (await confirmAction({
        title: `Sign out of ${name}?`,
        body: `Agents use this plan now. They stop thinking until another provider is in use.`,
        action: 'Sign out',
      }))
    if (!yes) return
    setBusy(true)
    setError('')
    try {
      const { revoked } = await ok(
        plan === 'chatgpt'
          ? api.DELETE('/api/provider/subscriptions/chatgpt')
          : api.DELETE('/api/provider/subscriptions/supergrok'),
      )
      await onChanged(
        revoked
          ? `Signed out of ${name}.`
          : `Signed out of ${name} here. ${PLANS[plan].vendor} did not confirm it, so also disconnect Cubebot in your ${name} settings.`,
      )
    } catch (caught) {
      setError(failure(caught))
      setBusy(false)
    }
  }
  return (
    <div className="plan__signed" data-problem={subscription.problem ?? 'none'}>
      <div className="plan__account">
        <span className="plan__who">
          <strong>{subscription.account}</strong>
          <span className="small faint">
            {subscription.mine ? 'Your plan' : `${subscription.owner}’s plan`}
            {inUse ? ' · in use' : ''}
          </span>
        </span>
        <button
          type="button"
          className="btn btn--ghost"
          disabled={busy}
          onClick={() => void signOut()}
        >
          <LogOut />
          {busy ? 'Signing out…' : 'Sign out'}
        </button>
      </div>
      {problem && (
        <p
          className={`notice ${problem.severe ? 'notice--danger' : ''}`}
          role="status"
        >
          {problem.text}
        </p>
      )}
      <Alert>{error}</Alert>
      {subscription.problem === 'sign_in_again' ? null : subscription.mine ? (
        children
      ) : (
        <p className="small faint">
          Only {subscription.owner} can make this plan the one agents use.
        </p>
      )}
    </div>
  )
}

function useStart<T>(path: () => Promise<T>) {
  const [started, setStarted] = useState<T | null>(null)
  const [error, setError] = useState('')
  const [busy, setBusy] = useState(false)
  const start = async () => {
    setBusy(true)
    setError('')
    try {
      setStarted(await path())
    } catch (caught) {
      setError(failure(caught))
    } finally {
      setBusy(false)
    }
  }
  return {
    started,
    start,
    error,
    setError,
    busy,
    reset: () => setStarted(null),
  }
}

function ChatgptSignIn({
  onChanged,
}: {
  onChanged: (message: string) => Promise<unknown>
}) {
  const sign = useStart(() =>
    ok(api.POST('/api/provider/subscriptions/chatgpt')),
  )
  const [landed, setLanded] = useState('')
  const [finishing, setFinishing] = useState(false)
  if (!sign.started)
    return (
      <div className="plan__start">
        <button
          type="button"
          className="btn btn--primary"
          disabled={sign.busy}
          onClick={() => void sign.start()}
        >
          <LogIn />
          {sign.busy ? 'Starting…' : 'Sign in with ChatGPT'}
        </button>
        <Alert>{sign.error}</Alert>
      </div>
    )
  const started = sign.started
  const finish = async () => {
    setFinishing(true)
    sign.setError('')
    const { data, error, response } = await api
      .POST('/api/provider/subscriptions/chatgpt/{sign_in}', {
        params: { path: { sign_in: started.sign_in } },
        body: { url: landed.trim() },
      })
      .catch(() => ({ data: undefined, error: undefined, response: null }))
    setFinishing(false)
    if (data) return onChanged(`Signed in to ChatGPT as ${data.account}.`)
    const code = (error as { error?: string } | undefined)?.error
    sign.setError(signInProblem('chatgpt', response?.status, code))
    if (response?.status === 422) sign.reset()
  }
  return (
    <form
      className="plan__steps"
      onSubmit={(event) => {
        event.preventDefault()
        void finish()
      }}
    >
      <ol>
        <li>
          <a
            className="btn btn--primary"
            href={started.url}
            target="_blank"
            rel="noopener noreferrer"
          >
            <ExternalLink />
            Open OpenAI sign-in
          </a>
          <span className="small faint">
            Sign in and allow Cubebot to use your plan.
          </span>
        </li>
        <li>
          <Field
            label="Paste the address OpenAI sends you to"
            hint="The page will not load; that is expected"
          >
            <input
              required
              type="url"
              inputMode="url"
              autoComplete="off"
              spellCheck={false}
              maxLength={8192}
              placeholder="http://127.0.0.1:1456/auth/callback?code=…"
              value={landed}
              onChange={(event) => setLanded(event.target.value)}
            />
          </Field>
        </li>
      </ol>
      <Alert>{sign.error}</Alert>
      <div className="actions">
        <button
          className="btn btn--primary"
          disabled={finishing || !landed.trim()}
        >
          {finishing ? 'Finishing…' : 'Finish sign-in'}
        </button>
        <button type="button" className="btn btn--ghost" onClick={sign.reset}>
          Cancel
        </button>
      </div>
    </form>
  )
}

type Device = components['schemas']['SupergrokSignIn']

function GrokSignIn({
  onChanged,
}: {
  onChanged: (message: string) => Promise<unknown>
}) {
  const [accepted, setAccepted] = useState(false)
  const sign = useStart(() =>
    ok(api.POST('/api/provider/subscriptions/supergrok')),
  )
  if (sign.started)
    return (
      <GrokApproval
        device={sign.started}
        onDone={onChanged}
        onCancel={(problem) => {
          sign.reset()
          sign.setError(problem)
        }}
      />
    )
  return (
    <div className="plan__start">
      <p className="notice notice--paint plan__risk">
        xAI has no sign-in for other apps yet. Cubebot signs in with the Grok
        CLI’s own app id and tells xAI it is the Grok CLI. xAI has not approved
        this for Cubebot and could restrict your account.
      </p>
      <label className="check">
        <input
          type="checkbox"
          checked={accepted}
          onChange={(event) => setAccepted(event.target.checked)}
        />
        <span>I accept the risk to my xAI account</span>
      </label>
      <button
        type="button"
        className="btn btn--primary"
        disabled={!accepted || sign.busy}
        onClick={() => void sign.start()}
      >
        <LogIn />
        {sign.busy ? 'Starting…' : 'Sign in with Grok'}
      </button>
      <Alert>{sign.error}</Alert>
    </div>
  )
}

/** Asks the orchestrator at xAI's interval until the person approves on xAI's page. */
function GrokApproval({
  device,
  onDone,
  onCancel,
}: {
  device: Device
  onDone: (message: string) => Promise<unknown>
  onCancel: (problem: string) => void
}) {
  const client = useQueryClient()
  // The parent's callbacks change identity on each render; polling must not restart with them.
  const settle = useRef({ onDone, onCancel })
  useEffect(() => {
    settle.current = { onDone, onCancel }
  })
  useEffect(() => {
    let live = true
    let timer = 0
    const poll = async () => {
      const { data, error, response } = await api
        .POST('/api/provider/subscriptions/supergrok/{sign_in}', {
          params: { path: { sign_in: device.sign_in } },
        })
        .catch(() => ({ data: undefined, error: undefined, response: null }))
      if (!live) return
      if (data) {
        await client.invalidateQueries({ queryKey: ['provider-subscriptions'] })
        return settle.current.onDone(`Signed in to Grok as ${data.account}.`)
      }
      if (response?.status === 202 || !response) {
        timer = window.setTimeout(() => void poll(), device.interval * 1000)
        return
      }
      const code = (error as { error?: string } | undefined)?.error
      settle.current.onCancel(signInProblem('supergrok', response.status, code))
    }
    timer = window.setTimeout(() => void poll(), device.interval * 1000)
    return () => {
      live = false
      window.clearTimeout(timer)
    }
  }, [device, client])
  return (
    <div className="plan__steps">
      <ol>
        <li>
          <span className="plan__code-row">
            <code className="plan__code" aria-label="Your code">
              {device.user_code}
            </code>
            <CopyButton text={device.user_code} label="Copy the code" />
          </span>
          <span className="small faint">
            Check xAI shows this same code before you approve.
          </span>
        </li>
        <li>
          <a
            className="btn btn--primary"
            href={device.verification_uri}
            target="_blank"
            rel="noopener noreferrer"
          >
            <ExternalLink />
            Approve on xAI
          </a>
        </li>
      </ol>
      <p className="plan__waiting small" role="status">
        <RefreshCw className="spin" aria-hidden="true" />
        Waiting for your approval. This page updates on its own.
      </p>
      <div className="actions">
        <button
          type="button"
          className="btn btn--ghost"
          onClick={() => onCancel('')}
        >
          Cancel
        </button>
      </div>
    </div>
  )
}
