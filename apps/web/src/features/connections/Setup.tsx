import { useQueryClient } from '@tanstack/react-query'
import { ClipboardPaste, ExternalLink, LogIn } from 'lucide-react'
import { useEffect, useRef, useState, type FormEvent } from 'react'
import { api, failure } from '../../api/client'
import type { Schemas } from '../../api/types'
import { Alert, CopyButton, Field } from '../../components/primitives'
import { answer, CONNECTIONS_KEY } from './api'
import {
  AUTH_LABEL,
  type Auth,
  type Catalog,
  type Connection,
  type Field as Setting,
  type Manifest,
  filled,
  slugProblem,
  suggestedSlug,
  value,
} from './model'

type Started = Schemas['SignInStarted']
type Done = (saved: Connection | null) => void

/** The wizard: the vendor's steps, the settings, then the sign-in itself. */
export function Setup({
  catalog,
  manifest,
  replacing,
  all,
  onDone,
  onCancel,
}: {
  catalog: Catalog
  manifest: Manifest
  replacing: Connection | null
  all: Connection[]
  onDone: Done
  onCancel: (() => void) | undefined
}) {
  const methods = replacing
    ? manifest.auth.filter((auth) => auth.kind === replacing.auth)
    : manifest.auth
  const [index, setIndex] = useState(0)
  const [callback, setCallback] = useState(false)
  const [started, setStarted] = useState<Started | null>(null)
  const auth = methods[index] ?? methods[0]
  if (!auth) return null
  const redirect =
    callback && catalog.callback_redirect
      ? catalog.callback_redirect
      : catalog.loopback_redirect
  const vendor = auth.kind === 'oauth2' || auth.kind === 'github_app'
  return (
    <section className="sheet__section setup" aria-labelledby="setup-title">
      <Heading
        title={
          replacing
            ? `Sign ${replacing.slug} in again`
            : `Connect ${manifest.name}`
        }
        onCancel={onCancel}
      />
      {methods.length > 1 && !started && (
        <div
          className="tabs setup__methods"
          role="radiogroup"
          aria-label="How to sign in"
        >
          {methods.map((method, at) => (
            <button
              key={method.kind}
              type="button"
              role="radio"
              aria-checked={at === index}
              onClick={() => setIndex(at)}
            >
              {AUTH_LABEL[method.kind]}
            </button>
          ))}
        </div>
      )}
      {!replacing && <Guide manifest={manifest} redirect={redirect} />}
      {vendor && !started && !replacing && (
        <Callback
          address={catalog.callback_redirect}
          checked={callback}
          onChange={setCallback}
        />
      )}
      {started ? (
        <Finish
          started={started}
          onDone={onDone}
          onRestart={() => setStarted(null)}
        />
      ) : (
        <Settings
          manifest={manifest}
          auth={auth}
          replacing={replacing}
          all={all}
          callback={callback}
          onStarted={setStarted}
          onDone={onDone}
        />
      )}
    </section>
  )
}

function Heading({
  title,
  onCancel,
}: {
  title: string
  onCancel: (() => void) | undefined
}) {
  return (
    <div className="sheet__section-head">
      <h3 id="setup-title">{title}</h3>
      {onCancel && (
        <button type="button" className="link-btn small" onClick={onCancel}>
          Cancel
        </button>
      )}
    </div>
  )
}

/** With a public address the vendor can return here instead of to a pasted loopback page. */
function Callback({
  address,
  checked,
  onChange,
}: {
  address: string | null | undefined
  checked: boolean
  onChange: (checked: boolean) => void
}) {
  if (!address) return null
  return (
    <label className="check setup__callback">
      <input
        type="checkbox"
        checked={checked}
        onChange={(event) => onChange(event.target.checked)}
      />
      <span>
        Return to this installation at <code>{address}</code> instead of pasting
        an address. Your app must allow it.
      </span>
    </label>
  )
}

function Guide({
  manifest,
  redirect,
}: {
  manifest: Manifest
  redirect: string
}) {
  if (manifest.guide.length === 0) return null
  return (
    <ol className="guide" aria-label="Steps">
      {manifest.guide.map((step, at) => (
        <li key={step.title} className="guide__step">
          <span className="guide__number" aria-hidden="true">
            {at + 1}
          </span>
          <div className="guide__text">
            <h4>{step.title}</h4>
            <p>{filled(step.body, redirect)}</p>
            {step.copy && (
              <div className="guide__copy">
                <code>{filled(step.copy, redirect)}</code>
                <CopyButton text={filled(step.copy, redirect)} label="Copy" />
              </div>
            )}
            {step.link && (
              <a
                className="btn btn--sm guide__link"
                href={step.link}
                target="_blank"
                rel="noreferrer"
              >
                <ExternalLink /> Open
              </a>
            )}
          </div>
        </li>
      ))}
    </ol>
  )
}

/** The settings form; pasted secrets save at once, vendor sign-ins start here. */
function Settings({
  manifest,
  auth,
  replacing,
  all,
  callback,
  onStarted,
  onDone,
}: {
  manifest: Manifest
  auth: Auth
  replacing: Connection | null
  all: Connection[]
  callback: boolean
  onStarted: (started: Started) => void
  onDone: Done
}) {
  const client = useQueryClient()
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  const taken = all.map((c) => c.slug)
  const [slug, setSlug] = useState(() => suggestedSlug(manifest, taken))
  const fields = replacing
    ? secretsOf(auth)
    : [...manifest.config, ...fieldsOf(auth)]
  const linkable = all.filter(
    (c) => auth.kind === 'linked' && auth.connectors.includes(c.connector),
  )
  const submit = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault()
    const problem = replacing ? '' : slugProblem(slug)
    if (problem) return setError(problem)
    setBusy(true)
    setError('')
    try {
      const form = new FormData(event.currentTarget)
      const result = await save(manifest, auth, replacing, {
        slug,
        form,
        fields,
        callback,
      })
      if (result.kind === 'saved') {
        await client.invalidateQueries({ queryKey: CONNECTIONS_KEY })
        onDone(result.connection)
      } else onStarted(result.started)
    } catch (caught) {
      setError(failure(caught))
    } finally {
      setBusy(false)
    }
  }
  return (
    <form className="form setup__form" onSubmit={(event) => void submit(event)}>
      {!replacing && (
        <Field label="Name" hint={`Tools are called ${slug || 'name'}__tool`}>
          <input
            value={slug}
            required
            maxLength={32}
            onChange={(e) => setSlug(e.target.value.toLowerCase())}
          />
        </Field>
      )}
      {fields.map((field) => (
        <SettingField key={field.key} field={field} />
      ))}
      {auth.kind === 'linked' && (
        <Linked connections={linkable} product={auth.product} />
      )}
      {!replacing && <Products manifest={manifest} />}
      <Alert>{error}</Alert>
      <div className="actions">
        <button
          className="btn btn--primary"
          disabled={busy || (auth.kind === 'linked' && linkable.length === 0)}
        >
          {auth.kind === 'oauth2' || auth.kind === 'github_app' ? (
            <LogIn />
          ) : null}
          {busy ? 'Checking…' : action(auth, replacing)}
        </button>
        <span className="small faint">
          {replacing
            ? ''
            : `Starts at ${manifest.level === 'full' ? 'Full, so agents can send' : manifest.level === 'write' ? 'Read and write' : 'Read'}; change it after connecting.`}
        </span>
      </div>
    </form>
  )
}

function action(auth: Auth, replacing: Connection | null) {
  if (auth.kind === 'oauth2') return replacing ? 'Sign in again' : 'Sign in'
  if (auth.kind === 'github_app') return 'Create the GitHub App'
  return replacing ? 'Replace and test' : 'Connect and test'
}

function fieldsOf(auth: Auth): Setting[] {
  return 'fields' in auth ? auth.fields : []
}

function secretsOf(auth: Auth): Setting[] {
  return fieldsOf(auth).filter((field) => field.kind.type === 'secret')
}

function SettingField({ field }: { field: Setting }) {
  const common = {
    name: field.key,
    required: field.required,
    placeholder: field.placeholder || undefined,
  }
  const control =
    field.kind.type === 'choice' ? (
      <select {...common} defaultValue="">
        {!field.required && <option value="">Not set</option>}
        {field.kind.choices.map((choice) => (
          <option key={choice.value} value={choice.value}>
            {choice.label}
          </option>
        ))}
      </select>
    ) : field.kind.type === 'lines' ? (
      <textarea {...common} rows={3} />
    ) : (
      <input
        {...common}
        type={
          field.kind.type === 'secret'
            ? 'password'
            : field.kind.type === 'number'
              ? 'number'
              : field.kind.type === 'url'
                ? 'url'
                : 'text'
        }
        autoComplete={field.kind.type === 'secret' ? 'new-password' : 'off'}
        spellCheck={false}
      />
    )
  return (
    <Field label={field.label} hint={field.help || undefined}>
      {control}
    </Field>
  )
}

function Linked({
  connections,
  product,
}: {
  connections: Connection[]
  product: string
}) {
  if (connections.length === 0)
    return (
      <p className="notice">
        Connect a Google account with {product} switched on first, then come
        back to sign in with it.
      </p>
    )
  return (
    <Field
      label="Sign in with"
      hint="Its access token is used; no password is stored here"
    >
      <select name="__linked" required>
        {connections.map((c) => (
          <option key={c.id} value={c.id}>
            {c.slug} {c.account ? `(${c.account})` : ''}
          </option>
        ))}
      </select>
    </Field>
  )
}

function Products({ manifest }: { manifest: Manifest }) {
  if (manifest.products.length === 0) return null
  return (
    <fieldset className="setup__products">
      <legend>Products</legend>
      <p className="small faint">
        Only these are asked for, and only their tools are offered.
      </p>
      <div className="setup__product-list">
        {manifest.products.map((product) => (
          <label key={product.id} className="check">
            <input
              type="checkbox"
              name="__products"
              value={product.id}
              defaultChecked
            />
            <span>{product.name}</span>
          </label>
        ))}
      </div>
    </fieldset>
  )
}

type Saved =
  | { kind: 'saved'; connection: Connection }
  | { kind: 'started'; started: Started }

async function save(
  manifest: Manifest,
  auth: Auth,
  replacing: Connection | null,
  input: { slug: string; form: FormData; fields: Setting[]; callback: boolean },
): Promise<Saved> {
  const { config, secrets } = collect(input.fields, input.form)
  const products = input.form.getAll('__products').map(String)
  if (manifest.products.length > 0 && !replacing) {
    if (products.length === 0) throw new Error('Choose at least one product.')
    config.products = products
  }
  const pasted = ['api_key', 'password', 'client_credentials', 'public']
  if (replacing && pasted.includes(auth.kind)) {
    const body = { secrets }
    const params = { path: { id: replacing.id } }
    const connection = await answer(
      api.PUT('/api/connections/{id}', { params, body }),
    )
    return { kind: 'saved', connection: connection as Connection }
  }
  if (auth.kind === 'oauth2' || auth.kind === 'github_app') {
    const body = {
      connector: manifest.id,
      slug: replacing?.slug ?? input.slug,
      auth: auth.kind,
      config: (replacing ? {} : config) as Record<string, never>,
      secrets,
      callback: input.callback,
      connection: replacing?.id ?? null,
    }
    const started = await answer(
      api.POST('/api/connections/sign-ins', { body }),
    )
    return { kind: 'started', started: started as Started }
  }
  const linked = input.form.get('__linked')
  const body = {
    connector: manifest.id,
    slug: input.slug,
    auth: auth.kind,
    config: config as Record<string, never>,
    secrets,
    linked: linked ? String(linked) : null,
  }
  const connection = await answer(api.POST('/api/connections', { body }))
  return { kind: 'saved', connection: connection as Connection }
}

function collect(fields: Setting[], form: FormData) {
  const config: Record<string, unknown> = {}
  const secrets: Record<string, string> = {}
  for (const field of fields) {
    const raw = String(form.get(field.key) ?? '')
    if (field.kind.type === 'secret') {
      if (raw) secrets[field.key] = raw
      continue
    }
    const typed = value(field, raw)
    if (typed !== undefined) config[field.key] = typed
  }
  return { config, secrets }
}

/** The vendor's half: open its page, then paste back, wait for approval, or come back here. */
function Finish({
  started,
  onDone,
  onRestart,
}: {
  started: Started
  onDone: Done
  onRestart: () => void
}) {
  if (started.kind === 'device')
    return <Device started={started} onDone={onDone} onRestart={onRestart} />
  const callback = !started.redirect_uri.startsWith('http://127.0.0.1')
  return (
    <div className="finish">
      <ol className="finish__steps">
        <li>
          {started.kind === 'form' ? (
            <GithubForm url={started.url} manifest={started.manifest} />
          ) : callback ? (
            <a className="btn btn--primary" href={started.url}>
              <LogIn /> Continue to sign in
            </a>
          ) : (
            <a
              className="btn btn--primary"
              href={started.url}
              target="_blank"
              rel="noreferrer"
            >
              <ExternalLink /> Open the sign-in page
            </a>
          )}
        </li>
        {!callback && (
          <li>
            <Paste
              sign_in={started.sign_in}
              redirect={started.redirect_uri}
              onDone={onDone}
            />
          </li>
        )}
      </ol>
      <button type="button" className="link-btn small" onClick={onRestart}>
        Start over
      </button>
    </div>
  )
}

/** GitHub takes the manifest as a posted form, so it opens in its own tab. */
function GithubForm({ url, manifest }: { url: string; manifest: string }) {
  return (
    <form action={url} method="post" target="_blank" className="finish__form">
      <input type="hidden" name="manifest" value={manifest} />
      <button className="btn btn--primary">
        <ExternalLink /> Create the app on GitHub
      </button>
      <span className="small faint">
        Confirm it there, then install it on the repositories it may use.
      </span>
    </form>
  )
}

function Paste({
  sign_in,
  redirect,
  onDone,
}: {
  sign_in: string
  redirect: string
  onDone: Done
}) {
  const client = useQueryClient()
  const [landed, setLanded] = useState('')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  const finish = async (event: FormEvent) => {
    event.preventDefault()
    setBusy(true)
    setError('')
    try {
      const params = { path: { sign_in } }
      const saved = await answer(
        api.POST('/api/connections/sign-ins/{sign_in}', {
          params,
          body: { url: landed.trim() },
        }),
      )
      await client.invalidateQueries({ queryKey: CONNECTIONS_KEY })
      onDone(saved ?? null)
    } catch (caught) {
      setError(failure(caught))
    } finally {
      setBusy(false)
    }
  }
  return (
    <form
      className="form finish__paste"
      onSubmit={(event) => void finish(event)}
    >
      <Field
        label="Paste the address the browser ended on"
        hint={`The page fails to load, which is expected; its address starts with ${redirect}`}
      >
        <input
          value={landed}
          required
          inputMode="url"
          spellCheck={false}
          placeholder={`${redirect}?code=…`}
          onChange={(event) => setLanded(event.target.value)}
        />
      </Field>
      <Alert>{error}</Alert>
      <div className="actions">
        <button className="btn btn--primary" disabled={busy || !landed.trim()}>
          <ClipboardPaste /> {busy ? 'Finishing…' : 'Finish signing in'}
        </button>
      </div>
    </form>
  )
}

function Device({
  started,
  onDone,
  onRestart,
}: {
  started: Extract<Started, { kind: 'device' }>
  onDone: Done
  onRestart: () => void
}) {
  const client = useQueryClient()
  const [error, setError] = useState('')
  // The sheet re-renders while this waits; polling must not restart with it.
  const done = useRef(onDone)
  useEffect(() => {
    done.current = onDone
  })
  useEffect(() => {
    let live = true
    const poll = async () => {
      try {
        const params = { path: { sign_in: started.sign_in } }
        const saved = await answer(
          api.POST('/api/connections/sign-ins/{sign_in}', { params, body: {} }),
        )
        if (!live) return
        if (saved) {
          await client.invalidateQueries({ queryKey: CONNECTIONS_KEY })
          return done.current(saved as Connection)
        }
        window.setTimeout(() => void poll(), started.interval * 1000)
      } catch (caught) {
        if (live) setError(failure(caught))
      }
    }
    const first = window.setTimeout(() => void poll(), started.interval * 1000)
    return () => {
      live = false
      window.clearTimeout(first)
    }
  }, [started, client])
  return (
    <div className="finish device">
      <p className="small">
        Open the sign-in page, enter this code, and approve Cubebot.
      </p>
      <div className="device__code">
        <code>{started.user_code}</code>
        <CopyButton text={started.user_code} label="Copy the code" />
      </div>
      <a
        className="btn btn--primary"
        href={started.url}
        target="_blank"
        rel="noreferrer"
      >
        <ExternalLink /> Open the sign-in page
      </a>
      {error ? (
        <>
          <Alert>{error}</Alert>
          <button type="button" className="link-btn small" onClick={onRestart}>
            Start over
          </button>
        </>
      ) : (
        <p className="small faint device__waiting" role="status">
          <span className="spin" aria-hidden="true" /> Waiting for your
          approval…
        </p>
      )}
    </div>
  )
}
