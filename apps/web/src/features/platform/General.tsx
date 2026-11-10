import { useQuery, useQueryClient } from '@tanstack/react-query'
import { type SubmitEvent, useState } from 'react'
import { api, failure, ok } from '../../api/client'
import { keys } from '../../app/queries'
import type { components } from '../../api/schema'
import { Alert, Field, Skeleton, Status } from '../../components/primitives'
import { fileSize } from '../files/viewer'
import { About } from './About'
import { bytesFrom, unitsOf } from './limits'
import { SaveBar, Section } from './Section'
import './platform.css'

type Settings = components['schemas']['SettingsView']
type Volume = components['schemas']['StorageView']

const GIB = 1024 ** 3
const MIB = 1024 ** 2

export function PlatformGeneral() {
  const settings = useQuery({
    queryKey: ['installation-settings'],
    queryFn: () => ok(api.GET('/api/installation/settings')),
  })
  return (
    <div className="settings">
      <About />
      {settings.isPending ? (
        <Skeleton lines={5} width="62ch" />
      ) : settings.isError ? (
        <Alert>
          The installation settings could not be read. Retry shortly.
        </Alert>
      ) : (
        <GeneralForm settings={settings.data} />
      )}
    </div>
  )
}

function GeneralForm({ settings }: { settings: Settings }) {
  const client = useQueryClient()
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  const [notice, setNotice] = useState('')
  const volume = settings.storage
  async function submit(event: SubmitEvent<HTMLFormElement>) {
    event.preventDefault()
    const data = new FormData(event.currentTarget)
    const name = String(data.get('name') ?? '').trim()
    setBusy(true)
    setError('')
    setNotice('')
    try {
      await ok(
        api.PUT('/api/installation/settings', {
          body: {
            name: name || null,
            storage_quota_bytes: bytesFrom(data.get('quota'), GIB),
            upload_limit_bytes: bytesFrom(data.get('upload'), MIB),
            revision: settings.revision,
          },
        }),
      )
      setNotice('Saved. The new values apply now.')
      await Promise.all([
        client.invalidateQueries({ queryKey: ['installation-settings'] }),
        client.invalidateQueries({ queryKey: keys.installation }),
      ])
    } catch (caught) {
      setError(failure(caught))
    } finally {
      setBusy(false)
    }
  }
  return (
    <form className="settings__form" onSubmit={submit}>
      <Section
        title="Name"
        description="What people see in the sidebar, the browser tab and invitations."
      >
        <Field
          label="Installation name"
          hint={`Empty uses “${settings.default_name}”`}
        >
          <input
            name="name"
            maxLength={64}
            placeholder={settings.default_name}
            defaultValue={settings.name ?? ''}
          />
        </Field>
      </Section>
      <Section
        title="Shared files"
        description={
          <p>
            How much of the shared-file volume people and agents may fill. A
            field left empty uses the deployment’s value, from the Helm chart or{' '}
            <code>.env</code>.
          </p>
        }
      >
        {volume ? (
          <StorageFields settings={settings} volume={volume} />
        ) : (
          <p className="small faint">
            This installation keeps no shared files, so it has no storage quota
            or upload limit to set.
          </p>
        )}
      </Section>
      <SaveBar>
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

function StorageFields({
  settings,
  volume,
}: {
  settings: Settings
  volume: Volume
}) {
  const ceiling =
    volume.capacity_bytes == null
      ? ''
      : `; at most ${fileSize(Math.floor(volume.capacity_bytes * 0.9))}, 90% of the volume`
  const upload = volume.default_upload_limit_bytes
  return (
    <>
      <VolumeMeter volume={volume} />
      <div className="settings__pair">
        <Field
          label="Storage quota (GiB)"
          hint={`Empty uses ${fileSize(volume.default_quota_bytes)}${ceiling}`}
        >
          <input
            name="quota"
            type="number"
            min={0.0625}
            step="any"
            inputMode="decimal"
            placeholder={unitsOf(volume.default_quota_bytes, GIB)}
            defaultValue={unitsOf(settings.storage_quota_bytes, GIB)}
          />
        </Field>
        <Field
          label="Largest upload (MiB)"
          hint={`Per file; empty uses ${upload == null ? 'no limit' : fileSize(upload)}`}
        >
          <input
            name="upload"
            type="number"
            min={1}
            step="any"
            inputMode="decimal"
            placeholder={upload == null ? 'No limit' : unitsOf(upload, MIB)}
            defaultValue={unitsOf(settings.upload_limit_bytes, MIB)}
          />
        </Field>
      </div>
    </>
  )
}

function VolumeMeter({ volume }: { volume: Volume }) {
  const share = Math.min(1, volume.used_bytes / Math.max(1, volume.quota_bytes))
  const state = share >= 1 ? 'over' : share >= 0.8 ? 'near' : 'fine'
  return (
    <div className="volume" data-state={state}>
      <p>
        <strong>{fileSize(volume.used_bytes)}</strong> of{' '}
        {fileSize(volume.quota_bytes)} in use
        {volume.capacity_bytes != null && (
          <span className="faint">
            {' '}
            · volume {fileSize(volume.capacity_bytes)}
          </span>
        )}
      </p>
      <span
        className="volume__track"
        role="meter"
        aria-label="Shared-file storage in use"
        aria-valuemin={0}
        aria-valuemax={volume.quota_bytes}
        aria-valuenow={volume.used_bytes}
      >
        <span style={{ transform: `scaleX(${share})` }} />
      </span>
    </div>
  )
}
