import { AlertCircle, Mail, X } from 'lucide-react'
import { useState } from 'react'
import { api, failure, ok } from '../../api/client'
import { Cube } from '../../components/Cube'
import { PersonAvatar } from '../../components/PersonAvatar'
import { Alert } from '../../components/primitives'
import { formatTime } from '../calendar/model'
import { Markdown } from '../chat/Markdown'
import '../chat/markdown.css'
import { useRefresh, type Meeting } from './api'

type Invitation = Meeting['invitations'][number]
type Utterance = Meeting['utterances'][number]

export function StateChip({ state }: { state: string }) {
  if (state === 'live')
    return (
      <span className="chip chip--live">
        <span className="live-dot" aria-hidden="true" /> Live
      </span>
    )
  return (
    <span className="chip">{state === 'ended' ? 'Ended' : 'Scheduled'}</span>
  )
}

function InviteeFace({ invitation }: { invitation: Invitation }) {
  if (invitation.kind === 'agent')
    return <Cube avatar={invitation.avatar ?? null} size={22} />
  if (invitation.kind === 'email')
    return (
      <span className="invitee__mail" aria-hidden="true">
        <Mail />
      </span>
    )
  return (
    <PersonAvatar
      account={invitation.account ?? undefined}
      name={invitation.name}
      className="member__photo"
    />
  )
}

function InviteeDetail({
  invitation,
  zone,
}: {
  invitation: Invitation
  zone: string
}) {
  if (invitation.email_error)
    return (
      <span
        className="member__kind invitee__error"
        title={invitation.email_error}
      >
        <AlertCircle aria-hidden="true" /> Not sent: {invitation.email_error}
      </span>
    )
  if (invitation.kind === 'email')
    return (
      <span className="member__kind">
        {invitation.emailed_at
          ? `Emailed at ${formatTime(invitation.emailed_at, zone)}`
          : 'Not emailed. They need the link.'}
      </span>
    )
  return (
    <span className="member__kind">
      {invitation.kind === 'agent' ? 'Agent' : 'Person'}
    </span>
  )
}

export function Invited({ meeting, zone }: { meeting: Meeting; zone: string }) {
  const refresh = useRefresh()
  const [busy, setBusy] = useState('')
  const [error, setError] = useState('')
  const remove = async (invitation: Invitation) => {
    setBusy(invitation.id)
    setError('')
    try {
      await ok(
        api.DELETE('/api/meetings/{id}/invitations/{invitation}', {
          params: { path: { id: meeting.id, invitation: invitation.id } },
        }),
      )
      await refresh()
    } catch (caught) {
      setError(failure(caught))
    } finally {
      setBusy('')
    }
  }
  if (!meeting.invitations.length)
    return <p className="members__empty">Nobody invited yet.</p>
  return (
    <>
      <ul className="members">
        {meeting.invitations.map((invitation) => (
          <li key={invitation.id} className="member">
            <InviteeFace invitation={invitation} />
            <span className="member__who">
              <span className="member__name">{invitation.name}</span>
              <InviteeDetail invitation={invitation} zone={zone} />
            </span>
            {meeting.owner && meeting.state !== 'ended' && (
              <button
                type="button"
                className="btn btn--ghost btn--icon btn--sm member__remove"
                aria-label={`Remove ${invitation.name}`}
                title="Remove"
                disabled={busy === invitation.id}
                onClick={() => void remove(invitation)}
              >
                <X />
              </button>
            )}
          </li>
        ))}
      </ul>
      <Alert>{error}</Alert>
    </>
  )
}

export function Summary({ meeting }: { meeting: Meeting }) {
  if (meeting.summary)
    return (
      <div className="meeting-summary">
        <Markdown text={meeting.summary} />
      </div>
    )
  return (
    <p className="meeting-detail__quiet">
      The summary is written when the meeting ends. It appears here a moment
      later.
    </p>
  )
}

function SpeakerFace({ line, meeting }: { line: Utterance; meeting: Meeting }) {
  if (line.kind === 'agent') {
    const agent = meeting.invitations.find(
      (invitation) =>
        invitation.kind === 'agent' && invitation.name === line.speaker,
    )
    return <Cube avatar={agent?.avatar ?? null} size={22} />
  }
  return <PersonAvatar name={line.speaker} className="member__photo" />
}

export function Transcript({
  meeting,
  zone,
}: {
  meeting: Meeting
  zone: string
}) {
  if (!meeting.utterances.length)
    return (
      <p className="meeting-detail__quiet">
        {meeting.state === 'ended'
          ? 'Nobody spoke in this meeting.'
          : 'What everyone says is written here as the meeting runs.'}
      </p>
    )
  return (
    <ol className="transcript" aria-label="Transcript">
      {meeting.utterances.map((line) => (
        <li
          key={line.sequence}
          className="transcript__line"
          data-agent={line.kind === 'agent' || undefined}
        >
          <SpeakerFace line={line} meeting={meeting} />
          <div className="transcript__body">
            <p className="transcript__head">
              <span className="transcript__who">{line.speaker}</span>
              <time className="transcript__at tabular" dateTime={line.at}>
                {formatTime(line.at, zone)}
              </time>
            </p>
            <p className="transcript__text">{line.text}</p>
          </div>
        </li>
      ))}
    </ol>
  )
}
