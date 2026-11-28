import { useQuery } from '@tanstack/react-query'
import { type SubmitEvent } from 'react'
import { admin } from '../../api/admin'
import {
  Alert,
  CopyButton,
  Field,
  Skeleton,
  Status,
  when,
} from '../../components/primitives'
import { useAdminAction } from './useAdminAction'

type Settings = Awaited<ReturnType<typeof admin.google>>

const SCOPE = 'https://www.googleapis.com/auth/admin.directory.group.readonly'

export function GoogleDirectory() {
  const google = useQuery({ queryKey: ['google'], queryFn: admin.google })
  if (google.isPending) return <Skeleton lines={4} />
  if (google.isError)
    return <Alert>The Google Workspace settings could not be read.</Alert>
  return <GoogleForm settings={google.data} />
}

function GoogleForm({ settings }: { settings: Settings }) {
  const action = useAdminAction(['google', 'groups', 'accounts'])
  function save(event: SubmitEvent<HTMLFormElement>) {
    event.preventDefault()
    const data = new FormData(event.currentTarget)
    const key = String(data.get('key') ?? '').trim()
    const customer = String(data.get('customer') ?? '').trim()
    void action.run(
      () =>
        admin.saveGoogle({
          enabled: data.has('enabled'),
          admin_email: String(data.get('admin') ?? '').trim() || null,
          customer: customer || null,
          // Untouched keeps the stored key.
          ...(key ? { service_account: key } : {}),
        }),
      'Saved.',
    )
  }
  const sync = () =>
    void action.run(async () => {
      const outcome = await admin.syncGoogle()
      if (outcome.problem) throw new Error(outcome.problem)
      return outcome
    }, 'Synced. Linked Google groups match Google now.')
  return (
    <form className="form" onSubmit={save}>
      <p className="small muted">
        Mirrors the members of Google groups you link under Groups and access.
        People are matched by the verified email of their single sign-on
        identity, so single sign-on with Google has to be on.
      </p>
      <ol className="small muted google__steps">
        <li>
          In Google Cloud, create a service account and a JSON key for it.
        </li>
        <li>
          In the Workspace Admin console, give it domain-wide delegation for
          this scope:
          <span className="sso__callback">
            <code>{SCOPE}</code>
            <CopyButton text={SCOPE} label="Copy scope" />
          </span>
        </li>
        <li>
          Paste the key here and name a Workspace administrator to act as.
        </li>
      </ol>
      <KeyField settings={settings} />
      <Field
        label="Workspace administrator"
        hint="The service account acts as them"
      >
        <input
          name="admin"
          type="email"
          autoComplete="off"
          placeholder="admin@example.com"
          defaultValue={settings.admin_email ?? ''}
        />
      </Field>
      <Field label="Customer ID" hint="my_customer is your own Workspace">
        <input
          name="customer"
          autoCapitalize="none"
          placeholder="my_customer"
          defaultValue={settings.customer}
        />
      </Field>
      <label className="check">
        <input
          type="checkbox"
          name="enabled"
          defaultChecked={settings.enabled}
        />
        Sync every 15 minutes
      </label>
      <SyncState settings={settings} />
      <Alert>{action.error}</Alert>
      <Status>{action.notice}</Status>
      <div className="actions">
        <button className="btn btn--primary btn--sm" disabled={action.busy}>
          Save Google Workspace
        </button>
        {settings.has_key && settings.admin_email && (
          <button
            type="button"
            className="btn btn--ghost btn--sm"
            disabled={action.busy}
            onClick={sync}
          >
            Sync now
          </button>
        )}
      </div>
    </form>
  )
}

function KeyField({ settings }: { settings: Settings }) {
  if (!settings.can_store_key)
    return (
      <p className="small muted">
        This installation has no credential key, so it cannot store a service
        account key. Set <code>CUBEBOT_CREDENTIAL_KEY_FILE</code> first.
      </p>
    )
  const hint = settings.has_key
    ? `Saved for ${settings.client_email ?? 'a service account'}. Leave empty to keep it`
    : 'Encrypted on the server and never shown again'
  return (
    <Field label="Service account key (JSON)" hint={hint}>
      <textarea
        name="key"
        rows={3}
        spellCheck={false}
        autoComplete="off"
        placeholder={'{ "type": "service_account", … }'}
      />
    </Field>
  )
}

function SyncState({ settings }: { settings: Settings }) {
  if (settings.problem)
    return <p className="notice notice--danger small">{settings.problem}</p>
  if (!settings.synced_at_unix) return null
  return (
    <p className="small faint">
      Last synced {when(new Date(settings.synced_at_unix * 1000).toISOString())}
      .
    </p>
  )
}
