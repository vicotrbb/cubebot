import type { Role } from '../../api/admin'

export const ROLES: { id: Role; label: string; meaning: string }[] = [
  {
    id: 'administrator',
    label: 'Administrator',
    meaning: 'Can do everything, on every agent and room',
  },
  {
    id: 'auditor',
    label: 'Auditor',
    meaning: 'Reads everything, changes nothing',
  },
  {
    id: 'member',
    label: 'Member',
    meaning: 'Uses what they are given and creates what the rules allow',
  },
  {
    id: 'guest',
    label: 'Guest',
    meaning: 'Uses only what is shared with them and creates nothing',
  },
]

export const roleLabel = (role: string | null | undefined) =>
  ROLES.find((item) => item.id === role)?.label ?? 'No role'

type Person = { id: string; owner: boolean; administrator: boolean }
type Viewer = { id: string; owner: boolean }
type Grouping = {
  name: string
  active: boolean
  members: string[]
  role?: string | null
}

/** Why the viewer may not change this person at all, as the server's rules have it. */
export function lockOf(person: Person, viewer: Viewer): string | null {
  if (person.id === viewer.id) return 'You'
  if (person.owner) return 'Installation owner'
  if (person.administrator && !viewer.owner)
    return 'Only the owner changes administrators'
  return null
}

/** The group a person's role comes from: the strongest role among their groups, as the server applies it. */
export function roleSource(account: string, groups: Grouping[]) {
  let found: Grouping | null = null
  for (const group of groups) {
    if (!group.active || !group.role || !group.members.includes(account))
      continue
    if (!found || rank(group.role) < rank(found.role)) found = group
  }
  return found
}

const rank = (role: string | null | undefined) =>
  ROLES.findIndex((item) => item.id === role)

/** Why the viewer may not change this person's role, or null when they may. */
export function roleLockOf(
  person: Person,
  viewer: Viewer,
  groups: Grouping[],
): string | null {
  const source = roleSource(person.id, groups)
  return lockOf(person, viewer) ?? (source ? `Set by ${source.name}` : null)
}

/** Roles the viewer may hand out: only the owner makes administrators. */
export const rolesFor = (owner: boolean) =>
  ROLES.filter((role) => owner || role.id !== 'administrator')

/** Who can own agents, rooms and teams: active administrators and members. */
export function heirs<T extends { id: string; active: boolean; role: string }>(
  people: T[],
  except?: string,
) {
  return people.filter(
    (person) =>
      person.active &&
      person.id !== except &&
      (person.role === 'administrator' || person.role === 'member'),
  )
}

export const resetLink = (origin: string, token: string) =>
  `${origin}/reset-password?token=${encodeURIComponent(token)}`
