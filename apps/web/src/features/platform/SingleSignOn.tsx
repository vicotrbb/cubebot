import { useQuery, useQueryClient } from '@tanstack/react-query'
import { type SubmitEvent, useState } from 'react'
import { ApiError, api, failure, ok } from '../../api/client'
import type { components } from '../../api/schema'
import {
  Alert,
  CopyButton,
  Field,
  Skeleton,
  Status,
} from '../../components/primitives'

type Settings = components['schemas']['SingleSignOnSettings']

const KEY = ['single-sign-on'] as const
const lines = (value: FormDataEntryValue | null) =>
  String(value ?? '')
    .split('\n')
    .map((line) => line.trim())
    .filter(Boolean)

function problem(caught: unknown) {
  if (caught instanceof ApiError && caught.status === 503)
    return 'The issuer could not be reached, or its discovery document is not valid. Nothing was changed.'
  if (caught instanceof ApiError && caught.status === 400)
    return 'Check the issuer URL, client ID and admission lists. Domains are bare names such as example.com.'
  return failure(caught)
}

export function SingleSignOnEditor() {
  const settings = useQuery({
    queryKey: KEY,
    queryFn: () => ok(api.GET('/api/identity/oidc/settings')),
  })
  if (settings.isPending) return <Skeleton lines={4} />
  if (settings.isError)
    return <Alert>Single sign-on settings could not be read.</Alert>
  if (settings.data.managed) return <Managed settings={settings.data} />
  if (!settings.data.redirect_uri)
    return (
      <p className="small muted">
        Set the installation’s public address first: <code>publicOrigin</code>{' '}
        in the Helm chart, or <code>CUBEBOT_PUBLIC_ORIGIN</code>. Single sign-on
        returns people there.
      </p>
    )
  return <ProviderForm settings={settings.data} />
}

function Managed({ settings }: { settings: Settings }) {
  return (
    <div className="form">
      <p className="small muted">
        Set by the deployment’s <code>CUBEBOT_OIDC_CONFIG</code>, so it is
        changed there, not here.
      </p>
      <dl className="sso__facts">
        <dt>Issuer</dt>
        <dd>{settings.issuer}</dd>
        <dt>Client ID</dt>
        <dd>
          <code>{settings.client_id}</code>
        </dd>
      </dl>
    </div>
  )
}

function ProviderForm({ settings }: { settings: Settings }) {
  const client = useQueryClient()
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  const [notice, setNotice] = useState('')
  const redirect = settings.redirect_uri ?? ''
  async function run(action: () => Promise<unknown>, done: string) {
    setBusy(true)
    setError('')
    setNotice('')
    try {
      await action()
      setNotice(done)
      await client.invalidateQueries({ queryKey: KEY })
    } catch (caught) {
      setError(problem(caught))
    } finally {
      setBusy(false)
    }
  }
  function save(event: SubmitEvent<HTMLFormElement>) {
    event.preventDefault()
    const data = new FormData(event.currentTarget)
    const secret = String(data.get('secret') ?? '')
    const body = {
      issuer: String(data.get('issuer') ?? '').trim(),
      client_id: String(data.get('client') ?? '').trim(),
      // Untouched keeps the saved secret.
      ...(secret ? { client_secret: secret } : {}),
      admit_subjects: lines(data.get('subjects')),
      admit_email_domains: lines(data.get('domains')),
    }
    void run(
      () => ok(api.PUT('/api/identity/oidc/settings', { body })),
      settings.enabled
        ? 'Saved. Sessions from the previous settings were signed out.'
        : 'Single sign-on is on. The sign-in page now offers it.',
    )
  }
  return (
    <form className="form" onSubmit={save}>
      <p className="small muted">
        Register this callback with your identity provider, then enter what it
        gives you. Cubebot checks the issuer before saving.
      </p>
      <div className="sso__callback">
        <code>{redirect}</code>
        <CopyButton text={redirect} label="Copy callback URL" />
      </div>
      <Field label="Issuer URL">
        <input
          name="issuer"
          type="url"
          required
          placeholder="https://id.example.com"
          defaultValue={settings.issuer ?? ''}
        />
      </Field>
      <Field label="Client ID">
        <input
          name="client"
          required
          maxLength={255}
          autoCapitalize="none"
          defaultValue={settings.client_id ?? ''}
        />
      </Field>
      <SecretField settings={settings} />
      <Field
        label="Admit these subjects"
        hint="One per line; each becomes a member at first sign-in"
      >
        <textarea
          name="subjects"
          rows={2}
          defaultValue={settings.admit_subjects.join('\n')}
        />
      </Field>
      <Field
        label="Admit verified emails from"
        hint="Domains, one per line, such as example.com"
      >
        <textarea
          name="domains"
          rows={2}
          defaultValue={settings.admit_email_domains.join('\n')}
        />
      </Field>
      <Alert>{error}</Alert>
      <Status>{notice}</Status>
      <div className="actions">
        <button className="btn btn--primary" disabled={busy}>
          {busy
            ? 'Checking…'
            : settings.enabled
              ? 'Save single sign-on'
              : 'Turn on single sign-on'}
        </button>
        {settings.enabled && (
          <button
            type="button"
            className="btn btn--ghost"
            disabled={busy}
            onClick={() =>
              void run(
                () => ok(api.DELETE('/api/identity/oidc/settings')),
                'Single sign-on is off. Its sessions were signed out.',
              )
            }
          >
            Turn off
          </button>
        )}
      </div>
    </form>
  )
}

function SecretField({ settings }: { settings: Settings }) {
  const hint = !settings.can_store_secret
    ? 'This installation has no credential key, so only a public client without a secret works'
    : settings.has_client_secret
      ? 'A secret is saved. Leave empty to keep it; it is never shown again'
      : 'Encrypted on the server; leave empty for a public client'
  return (
    <Field label="Client secret" hint={hint}>
      <input
        name="secret"
        type="password"
        autoComplete="off"
        disabled={!settings.can_store_secret}
      />
    </Field>
  )
}
