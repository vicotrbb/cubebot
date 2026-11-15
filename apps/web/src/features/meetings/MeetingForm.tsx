import { useQuery } from '@tanstack/react-query'
import { Link } from '@tanstack/react-router'
import { X } from 'lucide-react'
import { useId, useState, type ClipboardEvent, type KeyboardEvent } from 'react'
import { admin } from '../../api/admin'
import { can, type Schemas } from '../../api/types'
import { notRetired, useAgents, useSession } from '../../app/queries'
import { AddMember, MemberList } from '../../components/MemberPicker'
import {
  agentOptions,
  memberKey,
  personOptions,
  without,
  type MemberOption,
} from '../../components/members'
import { Field, Switch } from '../../components/primitives'
import { placeOf, userZone } from '../calendar/model'
import { useProfile } from '../shell/AboutYou'
import { connectionName, useEmailConnections } from './api'
import { addressesIn, isAddress, type When } from './schedule'
import '../agent/about.css'
import './screens.css'

type AgentMode = Schemas['AgentMode']

export const useZone = () => userZone(useProfile().data?.time_zone ?? '')

export function useInviteOptions() {
  const session = useSession().data
  const agents = useAgents()
  const administrator = Boolean(session?.administrator)
  const accounts = useQuery({
    queryKey: ['accounts'],
    queryFn: admin.accounts,
    enabled: administrator,
  })
  const sendable = notRetired(agents.data).filter(
    (agent) => session && can(agent, 'conversation_send', session),
  )
  return {
    agents: agentOptions(sendable),
    people: personOptions(
      (accounts.data ?? []).filter((person) => person.id !== session?.id),
    ),
  }
}

export const idsOf = (members: MemberOption[], kind: MemberOption['kind']) =>
  members.filter((member) => member.kind === kind).map((member) => member.id)

export function Invitees({
  legend,
  members,
  options,
  placeholder,
  onChange,
}: {
  legend: string
  members: MemberOption[]
  options: MemberOption[]
  placeholder: string
  onChange: (members: MemberOption[]) => void
}) {
  return (
    <fieldset className="member-field">
      <legend>{legend}</legend>
      {members.length > 0 && (
        <MemberList
          members={members}
          busy={false}
          removable={() => true}
          onRemove={(gone) =>
            onChange(
              members.filter((member) => memberKey(member) !== memberKey(gone)),
            )
          }
          empty=""
        />
      )}
      <AddMember
        options={without(options, members)}
        label="Invite"
        placeholder={placeholder}
        onAdd={(member) => onChange([...members, member])}
      />
    </fieldset>
  )
}

export function EmailInput({
  emails,
  onChange,
}: {
  emails: string[]
  onChange: (emails: string[]) => void
}) {
  const [text, setText] = useState('')
  const hint = useId()
  const add = (raw: string) => {
    const fresh = addressesIn(raw).filter((email) => !emails.includes(email))
    if (fresh.length) onChange([...new Set([...emails, ...fresh])])
    setText('')
  }
  const keyed = (event: KeyboardEvent<HTMLInputElement>) => {
    if ([',', ' ', ';', 'Enter'].includes(event.key) && text.trim()) {
      event.preventDefault()
      add(text)
    } else if (event.key === 'Backspace' && !text && emails.length)
      onChange(emails.slice(0, -1))
  }
  const pasted = (event: ClipboardEvent<HTMLInputElement>) => {
    event.preventDefault()
    add(text + ' ' + event.clipboardData.getData('text'))
  }
  const invalid = emails.filter((email) => !isAddress(email)).length
  return (
    <div className="email-field">
      <span>
        Email addresses
        <span className="hint">They get the link</span>
      </span>
      <div className="email-input">
        {emails.map((email) => (
          <span
            key={email}
            className={isAddress(email) ? 'chip' : 'chip chip--danger'}
          >
            {email}
            <button
              type="button"
              aria-label={`Remove ${email}`}
              onClick={() => onChange(emails.filter((each) => each !== email))}
            >
              <X />
            </button>
          </span>
        ))}
        <input
          type="email"
          inputMode="email"
          autoComplete="off"
          aria-label="Email addresses"
          aria-describedby={invalid ? hint : undefined}
          placeholder={emails.length ? '' : 'name@example.com'}
          value={text}
          onChange={(event) => setText(event.target.value)}
          onKeyDown={keyed}
          onPaste={pasted}
          onBlur={() => text.trim() && add(text)}
        />
      </div>
      {invalid > 0 && (
        <span id={hint} role="alert" className="email-input__problem">
          {invalid === 1
            ? 'One address does not look right. Fix or remove it.'
            : `${invalid} addresses do not look right. Fix or remove them.`}
        </span>
      )}
    </div>
  )
}

export const validEmails = (emails: string[]) => emails.every(isAddress)

export function MailerField({
  value,
  onChange,
}: {
  value: string
  onChange: (id: string) => void
}) {
  const mailers = useEmailConnections()
  const administrator = Boolean(useSession().data?.administrator)
  if (!mailers.data) return null
  if (!mailers.data.length)
    return (
      <p className="small faint">
        Emails go out through your own Gmail, Outlook or email connection that
        can send.{' '}
        {administrator && (
          <Link to="/platform/connections">Add a connection</Link>
        )}
      </p>
    )
  return (
    <Field label="Send emails from">
      <select value={value} onChange={(event) => onChange(event.target.value)}>
        {mailers.data.map((mailer) => (
          <option key={mailer.id} value={mailer.id}>
            {connectionName(mailer)}
          </option>
        ))}
      </select>
    </Field>
  )
}

export function useMailer() {
  const mailers = useEmailConnections()
  const [chosen, setChosen] = useState('')
  const value = chosen || mailers.data?.[0]?.id || ''
  return [value, setChosen] as const
}

export function WhenFields({
  when,
  zone,
  onChange,
}: {
  when: When
  zone: string
  onChange: (when: When) => void
}) {
  return (
    <div className="meeting-when">
      <Field label="Date" hint={`${placeOf(zone)} time`}>
        <input
          type="date"
          required
          value={when.day}
          onChange={(event) => onChange({ ...when, day: event.target.value })}
        />
      </Field>
      <Field label="Starts">
        <input
          type="time"
          required
          step={300}
          value={when.start}
          onChange={(event) => onChange({ ...when, start: event.target.value })}
        />
      </Field>
      <Field label="Ends">
        <input
          type="time"
          required
          step={300}
          value={when.end}
          onChange={(event) => onChange({ ...when, end: event.target.value })}
        />
      </Field>
    </div>
  )
}

const MODES: [AgentMode, string][] = [
  ['relevant', 'When relevant'],
  ['named', 'Only when named'],
]

export function ModeField({
  value,
  onChange,
}: {
  value: AgentMode
  onChange: (mode: AgentMode) => void
}) {
  const label = useId()
  return (
    <div className="field">
      <span id={label}>Agents speak</span>
      <div className="tabs segmented" role="group" aria-labelledby={label}>
        {MODES.map(([mode, name]) => (
          <button
            key={mode}
            type="button"
            aria-pressed={value === mode}
            onClick={() => onChange(mode)}
          >
            {name}
          </button>
        ))}
      </div>
    </div>
  )
}

export function OpenJoinField({
  checked,
  disabled = false,
  onChange,
}: {
  checked: boolean
  disabled?: boolean
  onChange: (open: boolean) => void
}) {
  return (
    <div className="capability">
      <div>
        <p className="capability__name">Anyone with the link joins directly</p>
        <p className="faint small">
          Off, guests wait until someone in the meeting lets them in. Signed-in
          members always join directly.
        </p>
      </div>
      <Switch
        checked={checked}
        disabled={disabled}
        label="Anyone with the link joins directly"
        onChange={onChange}
      />
    </div>
  )
}
