import { useNavigate } from '@tanstack/react-router'
import { CalendarPlus, Video } from 'lucide-react'
import { useState } from 'react'
import { api, failure, ok } from '../../api/client'
import type { Schemas } from '../../api/types'
import type { MemberOption } from '../../components/members'
import { Alert, Dialog, Field } from '../../components/primitives'
import { useMeetingSettings, useRefresh } from './api'
import {
  EmailInput,
  idsOf,
  Invitees,
  MailerField,
  ModeField,
  OpenJoinField,
  useInviteOptions,
  useMailer,
  useZone,
  validEmails,
  WhenFields,
} from './MeetingForm'
import { instantsOf, nextHalfHour, type When } from './schedule'

type Mode = 'now' | 'schedule'

export function NewMeeting({
  open,
  when,
  onClose,
}: {
  open: boolean
  when?: When | undefined
  onClose: () => void
}) {
  return (
    <Dialog open={open} title="New meeting" onClose={onClose}>
      <Choice when={when} onDone={onClose} />
    </Dialog>
  )
}

function Choice({
  when,
  onDone,
}: {
  when: When | undefined
  onDone: () => void
}) {
  const [mode, setMode] = useState<Mode>(when ? 'schedule' : 'now')
  return (
    <>
      <div
        className="tabs segmented segmented--wide"
        role="group"
        aria-label="When"
      >
        <button
          type="button"
          aria-pressed={mode === 'now'}
          onClick={() => setMode('now')}
        >
          <Video aria-hidden="true" /> Start now
        </button>
        <button
          type="button"
          aria-pressed={mode === 'schedule'}
          onClick={() => setMode('schedule')}
        >
          <CalendarPlus aria-hidden="true" /> Schedule
        </button>
      </div>
      {mode === 'now' ? (
        <StartNow onDone={onDone} />
      ) : (
        <ScheduleMeeting when={when} onDone={onDone} />
      )}
    </>
  )
}

function useCreate(onDone: () => void, stay = false) {
  const navigate = useNavigate()
  const refresh = useRefresh()
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  const create = async (body: () => Schemas['NewMeeting'], join: boolean) => {
    setBusy(true)
    setError('')
    try {
      const meeting = await ok(api.POST('/api/meetings', { body: body() }))
      void refresh()
      onDone()
      if (stay) return
      await navigate({
        to: '/meetings/$meetingId',
        params: { meetingId: meeting.id },
        search: { join: join || undefined },
      })
    } catch (caught) {
      setError(failure(caught))
      setBusy(false)
    }
  }
  return { busy, error, create }
}

function StartNow({ onDone }: { onDone: () => void }) {
  const options = useInviteOptions()
  const [title, setTitle] = useState('')
  const [agents, setAgents] = useState<MemberOption[]>([])
  const { busy, error, create } = useCreate(onDone)
  return (
    <form
      className="form meeting-form"
      onSubmit={(event) => {
        event.preventDefault()
        void create(
          () => ({
            title: title.trim() || 'Instant meeting',
            invite: {
              accounts: [],
              agents: idsOf(agents, 'agent'),
              emails: [],
            },
          }),
          true,
        )
      }}
    >
      <p className="meeting-form__lead">
        Opens the room now. Invite anyone else from inside it, or share its
        link.
      </p>
      <Field label="Title" hint="Optional">
        <input
          value={title}
          maxLength={200}
          placeholder="Instant meeting"
          autoFocus
          onChange={(event) => setTitle(event.target.value)}
        />
      </Field>
      {options.agents.length > 0 && (
        <Invitees
          legend="Bring agents along"
          members={agents}
          options={options.agents}
          placeholder="Search agents"
          onChange={setAgents}
        />
      )}
      <Alert>{error}</Alert>
      <div className="actions dialog__actions">
        <button className="btn btn--primary" disabled={busy}>
          <Video aria-hidden="true" />
          {busy ? 'Starting…' : 'Start meeting'}
        </button>
      </div>
    </form>
  )
}

export function ScheduleMeeting({
  when,
  stay = false,
  onDone,
  onCancel,
}: {
  when?: When | undefined
  stay?: boolean
  onDone: () => void
  onCancel?: () => void
}) {
  const zone = useZone()
  const options = useInviteOptions()
  const settings = useMeetingSettings()
  const [title, setTitle] = useState('')
  const [times, setTimes] = useState(() => when ?? nextHalfHour(zone))
  const [description, setDescription] = useState('')
  const [members, setMembers] = useState<MemberOption[]>([])
  const [emails, setEmails] = useState<string[]>([])
  const [mailer, setMailer] = useMailer()
  const [openJoin, setOpenJoin] = useState(false)
  const [mode, setMode] = useState<Schemas['AgentMode'] | null>(null)
  const { busy, error, create } = useCreate(onDone, stay)
  const agentMode = mode ?? settings.data?.agent_mode ?? 'relevant'
  const body = (): Schemas['NewMeeting'] => ({
    title: title.trim(),
    description: description.trim(),
    ...instantsOf(times, zone),
    open_join: openJoin,
    agent_mode: agentMode,
    invite: {
      accounts: idsOf(members, 'person'),
      agents: idsOf(members, 'agent'),
      emails,
    },
    email_connection: emails.length && mailer ? mailer : null,
  })
  const ready = Boolean(title.trim()) && validEmails(emails)
  return (
    <form
      className="form meeting-form"
      onSubmit={(event) => {
        event.preventDefault()
        void create(body, false)
      }}
    >
      <Field label="Title">
        <input
          value={title}
          required
          maxLength={200}
          autoFocus
          placeholder="Weekly sync"
          onChange={(event) => setTitle(event.target.value)}
        />
      </Field>
      <WhenFields when={times} zone={zone} onChange={setTimes} />
      <Field label="Description" hint="Optional">
        <textarea
          rows={2}
          maxLength={4000}
          value={description}
          onChange={(event) => setDescription(event.target.value)}
        />
      </Field>
      <Invitees
        legend="People and agents"
        members={members}
        options={[...options.agents, ...options.people]}
        placeholder={
          options.people.length ? 'Search agents and people' : 'Search agents'
        }
        onChange={setMembers}
      />
      <EmailInput emails={emails} onChange={setEmails} />
      {emails.length > 0 && <MailerField value={mailer} onChange={setMailer} />}
      <OpenJoinField checked={openJoin} onChange={setOpenJoin} />
      <ModeField value={agentMode} onChange={setMode} />
      <Alert>{error}</Alert>
      <div className="actions dialog__actions">
        {onCancel && (
          <button type="button" className="btn btn--ghost" onClick={onCancel}>
            Cancel
          </button>
        )}
        <button className="btn btn--primary" disabled={busy || !ready}>
          {busy ? 'Scheduling…' : 'Schedule meeting'}
        </button>
      </div>
    </form>
  )
}
