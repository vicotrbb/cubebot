import { useState } from 'react'
import { api, failure, ok } from '../../api/client'
import type { Schemas } from '../../api/types'
import {
  Alert,
  CopyButton,
  Field,
  Skeleton,
  Status,
  Switch,
} from '../../components/primitives'
import { SaveBar, Section } from '../platform/Section'
import { useMeetingSettings, useRefresh } from './api'
import { ModeField } from './MeetingForm'

type Settings = Schemas['MeetingSettings']
type Flag = 'agents_join' | 'agent_screens' | 'guests'

const FLAGS: [Flag, string, string][] = [
  [
    'agents_join',
    'Agents may join meetings',
    'Invited agents join as soon as the first person does, and answer when the meeting turns to them.',
  ],
  [
    'agent_screens',
    'Agents may share their screen',
    'An agent with a desktop can present it to everyone in the meeting.',
  ],
  [
    'guests',
    'Guests may join by link',
    'People without an account join with the meeting link. Off, links stop working everywhere.',
  ],
]

export function PlatformMeetings() {
  const settings = useMeetingSettings()
  if (settings.isPending) return <Skeleton lines={4} width="62ch" />
  if (settings.isError)
    return <Alert>The meeting settings could not be read. Retry shortly.</Alert>
  return <SettingsForm settings={settings.data} />
}

function SettingsForm({ settings }: { settings: Settings }) {
  const refresh = useRefresh()
  const [draft, setDraft] = useState(settings)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  const [notice, setNotice] = useState('')
  const set = (patch: Partial<Settings>) => setDraft({ ...draft, ...patch })
  const save = async () => {
    setBusy(true)
    setError('')
    setNotice('')
    try {
      await ok(
        api.PUT('/api/meetings/settings', {
          body: {
            agents_join: draft.agents_join,
            agent_screens: draft.agent_screens,
            guests: draft.guests,
            max_participants: draft.max_participants,
            agent_mode: draft.agent_mode,
          },
        }),
      )
      setNotice('Saved.')
      await refresh()
    } catch (caught) {
      setError(failure(caught))
    } finally {
      setBusy(false)
    }
  }
  const size = draft.max_participants
  const sized = Number.isInteger(size) && size >= 2 && size <= 24
  return (
    <form
      className="settings"
      onSubmit={(event) => {
        event.preventDefault()
        void save()
      }}
    >
      <Section
        title="Who takes part"
        description="What agents and guests may do in every meeting on this installation."
      >
        {FLAGS.map(([flag, name, about]) => (
          <div key={flag} className="capability">
            <div>
              <p className="capability__name">{name}</p>
              <p className="faint small">{about}</p>
            </div>
            <Switch
              checked={draft[flag]}
              label={name}
              onChange={(on) => set({ [flag]: on })}
            />
          </div>
        ))}
      </Section>
      <Section
        title="Size and manners"
        description="Applies to meetings created from now on; a meeting's host can change how its agents speak."
      >
        <Field label="Largest meeting" hint="People and agents, 2 to 24">
          <input
            type="number"
            inputMode="numeric"
            min={2}
            max={24}
            required
            value={Number.isNaN(size) ? '' : size}
            onChange={(event) =>
              set({ max_participants: event.target.valueAsNumber })
            }
          />
        </Field>
        <ModeField
          value={draft.agent_mode}
          onChange={(agent_mode) => set({ agent_mode })}
        />
      </Section>
      <Section
        title="Public address"
        description={
          <>
            <p>Meeting links start with this address.</p>
            <p>
              Kubernetes installs set it with the chart&rsquo;s{' '}
              <code>meetings.publicIngress</code>. Compose uses the console
              address.
            </p>
          </>
        }
      >
        {settings.public_origin ? (
          <div className="meeting-origin">
            <code>{settings.public_origin}</code>
            <CopyButton
              text={settings.public_origin}
              label="Copy public address"
            />
          </div>
        ) : (
          <p className="small faint">
            Not set. Links work only once the console has a public address.
          </p>
        )}
      </Section>
      <SaveBar>
        <Alert>{error}</Alert>
        <Status>{notice}</Status>
        <div className="actions">
          <button className="btn btn--primary" disabled={busy || !sized}>
            {busy ? 'Saving…' : 'Save meeting settings'}
          </button>
        </div>
      </SaveBar>
    </form>
  )
}
