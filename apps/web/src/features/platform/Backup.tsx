import { useQuery, useQueryClient } from '@tanstack/react-query'
import { DatabaseBackup, KeyRound, ShieldAlert } from 'lucide-react'
import { type ReactNode, type SubmitEvent, useState } from 'react'
import { api, failure, ok } from '../../api/client'
import type { components } from '../../api/schema'
import {
  Alert,
  Field,
  Skeleton,
  Status,
  confirmAction,
} from '../../components/primitives'
import { fileSize } from '../files/viewer'
import { STATUS, backupName, lastVerified, took } from './backups'
import { bytesFrom, unitsOf } from './limits'
import { SaveBar, Section } from './Section'
import './platform.css'

type View = components['schemas']['BackupView']
type Fields = components['schemas']['Fields']
type Summary = components['schemas']['BackupSummary']

const KEY = ['installation-backup']
const MIB = 1024 ** 2

export function PlatformBackup() {
  const backup = useQuery({
    queryKey: KEY,
    queryFn: () => ok(api.GET('/api/installation/backup')),
    // A running backup's status changes on its own.
    refetchInterval: (query) => (query.state.data?.running ? 3000 : false),
  })
  if (backup.isPending) return <Skeleton lines={6} width="62ch" />
  if (backup.isError)
    return <Alert>Backup settings could not be read. Retry shortly.</Alert>
  return (
    <div className="settings">
      <Section
        title="Backups"
        description="Copies of the database, shared files and every agent’s workspace, kept in a bucket you own."
      >
        <Overview view={backup.data} />
        <History backups={backup.data.backups} />
      </Section>
      <Terms view={backup.data} />
      <BackupForm key={backup.data.revision} view={backup.data} />
      {backup.data.restores.length > 0 && <Restores view={backup.data} />}
    </div>
  )
}

function configured(view: View) {
  const bucket = view.deployment.bucket ?? view.settings.bucket
  const key = view.deployment.access_key_id ?? view.settings.access_key_id
  const secret =
    view.deployment_secret_access_key || view.secret_access_key_saved
  return Boolean(bucket && key && secret)
}

function Overview({ view }: { view: View }) {
  const client = useQueryClient()
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  const last = lastVerified(view.backups)
  const ready = configured(view)
  const schedule = view.deployment.schedule ?? view.settings.schedule ?? 'daily'
  async function start() {
    setBusy(true)
    setError('')
    try {
      await ok(api.POST('/api/installation/backup/run'))
      await client.invalidateQueries({ queryKey: KEY })
    } catch (caught) {
      setError(failure(caught))
    } finally {
      setBusy(false)
    }
  }
  return (
    <div className="backup__overview">
      <DatabaseBackup aria-hidden="true" />
      <div>
        <h3>
          {view.running
            ? 'A backup is running'
            : last
              ? `Last verified backup: ${backupName(last.id)}`
              : 'No verified backup yet'}
        </h3>
        <p className="small faint">
          {ready
            ? schedule === 'off'
              ? 'Scheduled backups are off; start one here when you need it.'
              : 'One runs every day. The database, shared files and every agent’s workspace go to the bucket below.'
            : 'Set a bucket and its access key below to start backing up.'}
        </p>
        <Alert>{error}</Alert>
      </div>
      <button
        type="button"
        className="btn btn--primary"
        disabled={!ready || view.running || busy}
        onClick={start}
      >
        {view.running ? 'Backing up…' : 'Back up now'}
      </button>
    </div>
  )
}

function Terms({ view }: { view: View }) {
  return (
    <Section
      title="Nothing pauses"
      description={
        <p>
          Agents keep working and the console stays open while a backup runs.
          The database is read from one consistent moment, then shared files are
          copied, then each agent sends its workspace at low priority. An agent
          that keeps working meanwhile can come back with workspace files a few
          minutes newer than its conversation. A stopped or offline agent has no
          workspace in that backup; the backup says so.
        </p>
      }
    >
      <div className="backup__warning">
        <h3>
          <ShieldAlert aria-hidden="true" /> Backups are not encrypted
        </h3>
        <p>
          Anyone who can read the bucket can read every conversation, file and
          workspace in it, so its access policy is what protects them. Stored
          provider and connection credentials stay encrypted with this
          installation’s credential key, which no backup contains. A restore
          refuses to start without that key.
        </p>
        <CredentialKey view={view} />
      </div>
    </Section>
  )
}

function CredentialKey({ view }: { view: View }) {
  const client = useQueryClient()
  const [error, setError] = useState('')
  const elsewhere =
    view.deployment_source === 'Helm values' ? (
      <>
        the <code>credentials</code> Secret in this namespace holds it
      </>
    ) : (
      <>
        <code>cubebot key</code> prints it on this machine
      </>
    )
  if (!view.key.present)
    return (
      <p className="small">This installation has no credential key to keep.</p>
    )
  if (view.key.downloaded_at_unix != null) {
    const at = new Date(view.key.downloaded_at_unix * 1000).toLocaleString()
    return (
      <p className="small backup__key">
        <KeyRound aria-hidden="true" />
        <span>
          {view.key.downloaded_by ?? 'An administrator'} downloaded the key on{' '}
          {at}; it is offered once. To read it again, {elsewhere}.
        </span>
      </p>
    )
  }
  async function download() {
    const yes = await confirmAction({
      title: 'Download the credential key?',
      body: 'It is offered once and the download is recorded in the audit log. Keep it apart from the backups: anyone with both can read the stored credentials.',
      action: 'Download key',
    })
    if (!yes) return
    setError('')
    try {
      const text = await ok(
        api.POST('/api/installation/backup/key', { parseAs: 'text' }),
      )
      const link = document.createElement('a')
      link.href = URL.createObjectURL(new Blob([text], { type: 'text/plain' }))
      link.download = 'cubebot-credential-key.txt'
      link.click()
      URL.revokeObjectURL(link.href)
      await client.invalidateQueries({ queryKey: KEY })
    } catch (caught) {
      setError(failure(caught))
    }
  }
  return (
    <div className="backup__key">
      <button type="button" className="btn" onClick={download}>
        <KeyRound aria-hidden="true" /> Download the credential key
      </button>
      <p className="small faint">
        Once, for administrators. Otherwise {elsewhere}.
      </p>
      <Alert>{error}</Alert>
    </div>
  )
}

/** A setting the deployment fixes shows its value read-only, with where it comes from. */
function Setting({
  view,
  name,
  label,
  hint,
  shown,
  children,
}: {
  view: View
  name: keyof Fields
  label: string
  hint?: string
  shown?: (value: string | number) => string
  children: ReactNode
}) {
  const fixed = view.deployment[name]
  if (fixed == null)
    return (
      <Field label={label} hint={hint}>
        {children}
      </Field>
    )
  return (
    <Field label={label} hint={`Set in ${view.deployment_source}`}>
      <input
        readOnly
        aria-readonly="true"
        value={shown ? shown(fixed) : fixed}
      />
    </Field>
  )
}

function number(value: FormDataEntryValue | null) {
  const text = typeof value === 'string' ? value.trim() : ''
  return text && Number.isFinite(Number(text)) ? Math.round(Number(text)) : null
}

function text(value: FormDataEntryValue | null) {
  const trimmed = typeof value === 'string' ? value.trim() : ''
  return trimmed || null
}

function BackupForm({ view }: { view: View }) {
  const client = useQueryClient()
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  const [notice, setNotice] = useState('')
  const saved = view.settings
  const defaults = view.defaults
  async function submit(event: SubmitEvent<HTMLFormElement>) {
    event.preventDefault()
    const data = new FormData(event.currentTarget)
    const secret = text(data.get('secret'))
    setBusy(true)
    setError('')
    setNotice('')
    try {
      await ok(
        api.PUT('/api/installation/backup', {
          body: {
            endpoint: text(data.get('endpoint')),
            region: text(data.get('region')),
            bucket: text(data.get('bucket')),
            access_key_id: text(data.get('access_key_id')),
            schedule: text(data.get('schedule')),
            keep_daily: number(data.get('keep_daily')),
            keep_weekly: number(data.get('keep_weekly')),
            bandwidth_bytes: bytesFrom(data.get('bandwidth'), MIB),
            concurrency: number(data.get('concurrency')),
            ...(secret ? { secret_access_key: secret } : {}),
            revision: view.revision,
          },
        }),
      )
      setNotice('Saved. The next backup uses these settings.')
      await client.invalidateQueries({ queryKey: KEY })
    } catch (caught) {
      setError(failure(caught))
    } finally {
      setBusy(false)
    }
  }
  const secretFixed = view.deployment_secret_access_key
  return (
    <form className="settings__form backup__form" onSubmit={submit}>
      <Section
        title="Bucket"
        description="Any S3-compatible bucket: AWS S3, Cloudflare R2, Backblaze B2 or MinIO. Give this installation a bucket of its own."
      >
        <Setting
          view={view}
          name="endpoint"
          label="Endpoint"
          hint="Empty for AWS S3"
        >
          <input
            name="endpoint"
            type="url"
            placeholder="https://s3.eu-west-1.amazonaws.com"
            defaultValue={saved.endpoint ?? ''}
          />
        </Setting>
        <div className="settings__pair">
          <Setting view={view} name="bucket" label="Bucket">
            <input
              name="bucket"
              autoComplete="off"
              spellCheck={false}
              defaultValue={saved.bucket ?? ''}
            />
          </Setting>
          <Setting view={view} name="region" label="Region" hint="auto for R2">
            <input
              name="region"
              placeholder={defaults.region}
              spellCheck={false}
              defaultValue={saved.region ?? ''}
            />
          </Setting>
        </div>
        <div className="settings__pair">
          <Setting view={view} name="access_key_id" label="Access key ID">
            <input
              name="access_key_id"
              autoComplete="off"
              spellCheck={false}
              defaultValue={saved.access_key_id ?? ''}
            />
          </Setting>
          {secretFixed ? (
            <Field
              label="Secret access key"
              hint={`Set in ${view.deployment_source}`}
            >
              <input readOnly aria-readonly="true" value="••••••••••••" />
            </Field>
          ) : (
            <Field
              label="Secret access key"
              hint={view.secret_access_key_saved ? 'Saved' : undefined}
            >
              <input
                name="secret"
                type="password"
                autoComplete="new-password"
                disabled={!view.can_save_secret}
                placeholder={
                  view.secret_access_key_saved ? 'Type to replace it' : ''
                }
              />
            </Field>
          )}
        </div>
      </Section>
      <Section
        title="Schedule and retention"
        description="Each day and each week keeps its newest backup. A shared file leaves the bucket only once no kept backup needs it."
      >
        <div className="settings__pair">
          <Setting view={view} name="schedule" label="Schedule">
            <select name="schedule" defaultValue={saved.schedule ?? ''}>
              <option value="">Daily</option>
              <option value="off">Off: only Back up now</option>
            </select>
          </Setting>
          <Setting
            view={view}
            name="keep_daily"
            label="Days kept"
            hint={`Empty: ${defaults.keep_daily}`}
          >
            <input
              name="keep_daily"
              type="number"
              min={1}
              max={365}
              inputMode="numeric"
              placeholder={String(defaults.keep_daily)}
              defaultValue={saved.keep_daily ?? ''}
            />
          </Setting>
          <Setting
            view={view}
            name="keep_weekly"
            label="Weeks kept"
            hint={`Empty: ${defaults.keep_weekly}`}
          >
            <input
              name="keep_weekly"
              type="number"
              min={0}
              max={520}
              inputMode="numeric"
              placeholder={String(defaults.keep_weekly)}
              defaultValue={saved.keep_weekly ?? ''}
            />
          </Setting>
        </div>
      </Section>
      <Section
        title="Limits"
        description="Shared by the database, shared files and workspaces, so a backup never takes more of the network than this."
      >
        <div className="settings__pair">
          <Setting
            view={view}
            name="bandwidth_bytes"
            label="Bandwidth (MiB/s)"
            hint={`Empty: ${unitsOf(defaults.bandwidth_bytes, MIB)}`}
            shown={(bytes) => `${unitsOf(Number(bytes), MIB)} MiB/s`}
          >
            <input
              name="bandwidth"
              type="number"
              min={1}
              step="any"
              inputMode="decimal"
              placeholder={unitsOf(defaults.bandwidth_bytes, MIB)}
              defaultValue={unitsOf(saved.bandwidth_bytes, MIB)}
            />
          </Setting>
          <Setting
            view={view}
            name="concurrency"
            label="Transfers at once"
            hint={`Empty: ${defaults.concurrency}`}
          >
            <input
              name="concurrency"
              type="number"
              min={1}
              max={16}
              inputMode="numeric"
              placeholder={String(defaults.concurrency)}
              defaultValue={saved.concurrency ?? ''}
            />
          </Setting>
        </div>
      </Section>
      <SaveBar>
        {!view.can_save_secret && !secretFixed && (
          <p className="small faint">
            This installation has no credential key, so a secret can only come
            from {view.deployment_source}.
          </p>
        )}
        <Alert>{error}</Alert>
        <Status>{notice}</Status>
        <div className="actions">
          <button className="btn btn--primary" disabled={busy}>
            {busy ? 'Saving…' : 'Save settings'}
          </button>
        </div>
      </SaveBar>
    </form>
  )
}

function History({ backups }: { backups: Summary[] }) {
  return (
    <div className="backup__history">
      <h3>History</h3>
      {backups.length === 0 ? (
        <p className="small faint">No backup has run yet.</p>
      ) : (
        <ol>
          {backups.map((backup) => (
            <BackupRow key={backup.id} backup={backup} />
          ))}
        </ol>
      )}
    </div>
  )
}

function BackupRow({ backup }: { backup: Summary }) {
  const status = STATUS[backup.status] ?? { label: backup.status, tone: '' }
  const facts = [
    backup.bytes > 0 ? fileSize(backup.bytes) : '',
    took(backup.started_at_unix, backup.finished_at_unix),
    backup.requested_by ? `started by ${backup.requested_by}` : 'scheduled',
  ].filter(Boolean)
  return (
    <li className="backup__row" data-status={backup.status}>
      <div className="backup__line">
        <strong>{backupName(backup.id)}</strong>
        <span className={`chip${status.tone ? ` chip--${status.tone}` : ''}`}>
          {status.label}
        </span>
        <span className="small faint tabular">{facts.join(' · ')}</span>
        <code className="backup__id">{backup.id}</code>
      </div>
      {backup.error && <p className="small backup__error">{backup.error}</p>}
      {backup.missing.length > 0 && (
        <details className="backup__missing">
          <summary className="small">
            {backup.missing.length === 1
              ? '1 part missing'
              : `${backup.missing.length} parts missing`}
          </summary>
          <ul>
            {backup.missing.map((missing) => (
              <li key={missing.part} className="small">
                <strong>{missing.part}</strong>: {missing.reason}
              </li>
            ))}
          </ul>
        </details>
      )}
    </li>
  )
}

function Restores({ view }: { view: View }) {
  return (
    <Section
      title="Restored workspaces"
      description="Each agent fetches its workspace from the backup before it takes work. Connections and sign-ins came back as they were and may have expired."
    >
      <ul className="backup__rows">
        {view.restores.map((restore) => (
          <li key={restore.agent} className="backup__row">
            <div className="backup__line">
              <strong>{restore.agent_name}</strong>
              {restore.restored_at_unix != null ? (
                <span className="chip chip--sage">Restored</span>
              ) : restore.error ? (
                <span className="chip chip--danger">Failed, retrying</span>
              ) : (
                <span className="chip chip--paint">Waiting for the agent</span>
              )}
              <code className="backup__id">{restore.backup}</code>
            </div>
            {restore.error && (
              <p className="small backup__error">{restore.error}</p>
            )}
          </li>
        ))}
      </ul>
    </Section>
  )
}
