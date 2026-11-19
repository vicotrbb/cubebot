import { AlertCircle, Mail, X } from 'lucide-react'
import { useState } from 'react'
import { api, failure, ok } from '../../api/client'
import { Cube } from '../../components/Cube'
import { PersonAvatar } from '../../components/PersonAvatar'
import { Alert } from '../../components/primitives'
import '../../components/member-picker.css'
import { formatTime } from '../calendar/model'
import { Markdown } from '../chat/Markdown'
import '../chat/markdown.css'
import { useRefresh, type Meeting } from './api'
import { peopleOf, type Person } from './people'
import { minutesWords } from './schedule'

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

function Face({ person }: { person: Person }) {
  const invitation = person.invitation
  if (person.kind === 'agent')
    return <Cube avatar={invitation?.avatar ?? null} size={22} />
  if (person.kind === 'email')
    return (
      <span className="invitee__mail" aria-hidden="true">
        <Mail />
      </span>
    )
  return (
    <PersonAvatar
      account={person.account}
      name={person.name}
      className="member__photo"
    />
  )
}

const KINDS: Record<string, string> = {
  agent: 'Agent',
  account: 'Person',
  guest: 'Guest',
  email: 'Email',
}

function presence(person: Person, meeting: Meeting, zone: string) {
  if (person.joined && person.here)
    return `In the meeting · joined ${formatTime(person.joined, zone)}`
  if (person.joined)
    return `Joined ${formatTime(person.joined, zone)} · ${minutesWords(person.minutes)}`
  if (meeting.state === 'ended') return 'Invited, did not join'
  const invitation = person.invitation
  if (invitation?.kind !== 'email') return 'Invited'
  return invitation.emailed_at
    ? `Emailed at ${formatTime(invitation.emailed_at, zone)}`
    : 'Not emailed. They need the link.'
}

function PersonDetail({
  person,
  meeting,
  zone,
}: {
  person: Person
  meeting: Meeting
  zone: string
}) {
  const error = person.invitation?.email_error
  if (error && !person.joined)
    return (
      <span className="member__kind invitee__error" title={error}>
        <AlertCircle aria-hidden="true" /> Not sent: {error}
      </span>
    )
  return (
    <span className="member__kind">
      {person.kind !== 'email' && `${KINDS[person.kind] ?? 'Person'} · `}
      {presence(person, meeting, zone)}
    </span>
  )
}

export function People({ meeting, zone }: { meeting: Meeting; zone: string }) {
  const refresh = useRefresh()
  const [busy, setBusy] = useState('')
  const [error, setError] = useState('')
  const people = peopleOf(meeting)
  const removable = meeting.owner && meeting.state !== 'ended'
  const remove = async ({ invitation }: Person) => {
    if (!invitation) return
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
  if (!people.length)
    return <p className="members__empty">Nobody invited yet.</p>
  return (
    <>
      <ul className="members">
        {people.map((person) => (
          <li
            key={person.key}
            className="member meeting-attendee"
            data-absent={
              (meeting.state === 'ended' && !person.joined) || undefined
            }
            data-here={person.here || undefined}
          >
            <Face person={person} />
            <span className="member__who">
              <span className="member__name">
                {person.kind === 'guest'
                  ? person.name.replace(/ \(guest\)$/, '')
                  : person.name}
              </span>
              <PersonDetail person={person} meeting={meeting} zone={zone} />
            </span>
            {removable && person.invitation && (
              <button
                type="button"
                className="btn btn--ghost btn--icon btn--sm member__remove"
                aria-label={`Remove ${person.name}`}
                title="Remove"
                disabled={busy === person.invitation.id}
                onClick={() => void remove(person)}
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
    <ol className="record-lines" aria-label="Transcript">
      {meeting.utterances.map((line) => (
        <li
          key={line.sequence}
          className="record-lines__line"
          data-agent={line.kind === 'agent' || undefined}
        >
          <SpeakerFace line={line} meeting={meeting} />
          <div className="record-lines__body">
            <p className="record-lines__head">
              <span className="record-lines__who">{line.speaker}</span>
              <time className="record-lines__at tabular" dateTime={line.at}>
                {formatTime(line.at, zone)}
              </time>
            </p>
            <p className="record-lines__text">{line.text}</p>
          </div>
        </li>
      ))}
    </ol>
  )
}
