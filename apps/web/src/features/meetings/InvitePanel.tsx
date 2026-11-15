import { useState } from 'react'
import { api, failure, ok } from '../../api/client'
import { memberKey, type MemberOption } from '../../components/members'
import { Alert, CopyButton, Status } from '../../components/primitives'
import { useRefresh, type Meeting } from './api'
import {
  EmailInput,
  idsOf,
  Invitees,
  MailerField,
  OpenJoinField,
  useInviteOptions,
  useMailer,
  validEmails,
} from './MeetingForm'

export function invitedMembers(meeting: Meeting): MemberOption[] {
  return meeting.invitations.flatMap((invitation): MemberOption[] => {
    if (invitation.account)
      return [{ kind: 'person', id: invitation.account, name: invitation.name }]
    if (invitation.agent)
      return [{ kind: 'agent', id: invitation.agent, name: invitation.name }]
    return []
  })
}

function sentWords(count: number, agents: number) {
  const invited = count === 1 ? 'Invited 1.' : `Invited ${count}.`
  return agents ? `${invited} Agents join in a moment.` : invited
}

export function InvitePanel({ meeting }: { meeting: Meeting }) {
  const options = useInviteOptions()
  const refresh = useRefresh()
  const [members, setMembers] = useState<MemberOption[]>([])
  const [emails, setEmails] = useState<string[]>([])
  const [mailer, setMailer] = useMailer()
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  const [notice, setNotice] = useState('')
  const taken = new Set(invitedMembers(meeting).map(memberKey))
  const choices = [...options.agents, ...options.people].filter(
    (option) => !taken.has(memberKey(option)),
  )
  const count = members.length + emails.length
  const send = async () => {
    setBusy(true)
    setError('')
    setNotice('')
    try {
      await ok(
        api.POST('/api/meetings/{id}/invitations', {
          params: { path: { id: meeting.id } },
          body: {
            invite: {
              accounts: idsOf(members, 'person'),
              agents: idsOf(members, 'agent'),
              emails,
            },
            email_connection: emails.length && mailer ? mailer : null,
          },
        }),
      )
      setNotice(sentWords(count, idsOf(members, 'agent').length))
      setMembers([])
      setEmails([])
      await refresh()
    } catch (caught) {
      setError(failure(caught))
    } finally {
      setBusy(false)
    }
  }
  return (
    <section
      className="meeting-panel__section invite-panel"
      aria-label="Invite"
    >
      <h3>Invite</h3>
      {meeting.link && (
        <div className="invite-panel__link">
          <span className="invite-panel__url">{meeting.link}</span>
          <CopyButton text={meeting.link} label="Copy meeting link" />
        </div>
      )}
      <Invitees
        legend="People and agents"
        members={members}
        options={choices}
        placeholder={
          options.people.length ? 'Search agents and people' : 'Search agents'
        }
        onChange={setMembers}
      />
      <EmailInput emails={emails} onChange={setEmails} />
      {emails.length > 0 && <MailerField value={mailer} onChange={setMailer} />}
      <Alert>{error}</Alert>
      <Status>{notice}</Status>
      {count > 0 && (
        <button
          type="button"
          className="btn btn--primary btn--sm invite-panel__send"
          disabled={busy || !validEmails(emails)}
          onClick={() => void send()}
        >
          {busy
            ? 'Sending…'
            : `Send ${count === 1 ? 'invitation' : `${count} invitations`}`}
        </button>
      )}
      {meeting.owner && <OpenJoin meeting={meeting} />}
    </section>
  )
}

export function OpenJoin({ meeting }: { meeting: Meeting }) {
  const refresh = useRefresh()
  const [error, setError] = useState('')
  const [busy, setBusy] = useState(false)
  const change = async (open_join: boolean) => {
    setBusy(true)
    setError('')
    try {
      await ok(
        api.PATCH('/api/meetings/{id}', {
          params: { path: { id: meeting.id } },
          body: { open_join },
        }),
      )
      await refresh()
    } catch (caught) {
      setError(failure(caught))
    } finally {
      setBusy(false)
    }
  }
  return (
    <>
      <OpenJoinField
        checked={meeting.open_join}
        disabled={busy}
        onChange={(open) => void change(open)}
      />
      <Alert>{error}</Alert>
    </>
  )
}
