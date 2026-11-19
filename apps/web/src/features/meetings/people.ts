import type { Meeting } from './api'

type Invitation = Meeting['invitations'][number]
type Visit = Meeting['attendance'][number]

export type Person = {
  key: string
  name: string
  kind: string
  account?: string | undefined
  invitation?: Invitation
  joined?: string
  minutes: number
  here: boolean
}

const keyOf = (kind: string, id: string | null | undefined, name: string) =>
  `${kind}:${id ?? name}`

const minutesOf = (visit: Visit, until: number) =>
  ((visit.left_at ? Date.parse(visit.left_at) : until) -
    Date.parse(visit.joined_at)) /
  60_000

function invited(invitation: Invitation): Person {
  const id = invitation.account ?? invitation.agent ?? invitation.id
  return {
    key: keyOf(invitation.kind, id, invitation.name),
    name: invitation.name,
    kind: invitation.kind,
    account: invitation.account ?? undefined,
    invitation,
    minutes: 0,
    here: false,
  }
}

function attend(person: Person, visit: Visit, until: number, live: boolean) {
  person.joined ??= visit.joined_at
  person.minutes += Math.max(0, minutesOf(visit, until))
  person.here ||= live && !visit.left_at
}

export function peopleOf(meeting: Meeting, now = Date.now()): Person[] {
  const people = new Map<string, Person>()
  for (const invitation of meeting.invitations) {
    const person = invited(invitation)
    people.set(person.key, person)
  }
  const until = meeting.ended_at ? Date.parse(meeting.ended_at) : now
  for (const visit of meeting.attendance) {
    const key = keyOf(visit.kind, visit.account ?? visit.agent, visit.name)
    const person = people.get(key) ?? {
      key,
      name: visit.name,
      kind: visit.kind,
      account: visit.account ?? undefined,
      minutes: 0,
      here: false,
    }
    attend(person, visit, until, meeting.state === 'live')
    people.set(key, person)
  }
  return [...people.values()].sort(
    (a, b) =>
      Number(b.here) - Number(a.here) ||
      Number(Boolean(b.joined)) - Number(Boolean(a.joined)) ||
      (a.joined ?? '').localeCompare(b.joined ?? ''),
  )
}
