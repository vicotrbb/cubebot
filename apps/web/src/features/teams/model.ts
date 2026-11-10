import type { components } from '../../api/schema'

export type Team = components['schemas']['Team']
export type TeamMember = components['schemas']['TeamMember']
export type SaveTeam = components['schemas']['SaveTeam']
export type SaveMember = components['schemas']['SaveMember']

export function managerOf(team: { members: SaveMember[] }) {
  return team.members.find((member) => member.reports_to == null)
}

/** The members who report to `agent` in this team. */
export function reportsOf<T extends SaveMember>(members: T[], agent: string) {
  return members.filter((member) => member.reports_to === agent)
}

export function toSave(team: Team): SaveTeam {
  return {
    name: team.name,
    description: team.description,
    instructions: team.instructions,
    parent: team.parent ?? null,
    reports_to: team.reports_to ?? null,
    members: team.members.map(({ agent, title, role, reports_to }) => ({
      agent,
      title,
      role,
      reports_to: reports_to ?? null,
    })),
  }
}

/** Makes `agent` the manager; the previous manager then reports to it. */
export function makeManager(team: SaveTeam, agent: string): SaveTeam {
  const previous = managerOf(team)?.agent
  return {
    ...team,
    members: team.members.map((member) => {
      if (member.agent === agent) return { ...member, reports_to: null }
      if (member.agent === previous) return { ...member, reports_to: agent }
      return member
    }),
  }
}

/** Removes a member; whoever reported to it reports to its own manager. The manager stays. */
export function removeMember(team: SaveTeam, agent: string): SaveTeam {
  const gone = team.members.find((member) => member.agent === agent)
  if (!gone || gone.reports_to == null) return team
  return {
    ...team,
    members: team.members
      .filter((member) => member.agent !== agent)
      .map((member) =>
        member.reports_to === agent
          ? { ...member, reports_to: gone.reports_to ?? null }
          : member,
      ),
  }
}

/** Members `agent` may report to: anyone not below it, so lines never loop. */
export function bossChoices(team: SaveTeam, agent: string) {
  const below = new Set([agent])
  let grew = true
  while (grew) {
    grew = false
    for (const member of team.members)
      if (
        member.reports_to &&
        below.has(member.reports_to) &&
        !below.has(member.agent)
      ) {
        below.add(member.agent)
        grew = true
      }
  }
  return team.members.filter((member) => !below.has(member.agent))
}

/** Teams at any depth under `id`, which cannot become its parent. */
export function teamsBelow(teams: Team[], id: string) {
  const found = new Set<string>([id])
  let grew = true
  while (grew) {
    grew = false
    for (const team of teams)
      if (team.parent && found.has(team.parent) && !found.has(team.id)) {
        found.add(team.id)
        grew = true
      }
  }
  return found
}

/** What keeps a draft from saving, in the words the form shows; empty when it can save. */
export function problems(team: SaveTeam, others: Team[]): string[] {
  const found: string[] = []
  const name = team.name.trim().toLowerCase()
  if (!name) found.push('Name the team.')
  else if (others.some((other) => other.name.trim().toLowerCase() === name))
    found.push('Another team already has this name.')
  if (
    team.members.length > 0 &&
    team.members.filter((m) => m.reports_to == null).length !== 1
  )
    found.push('Choose one manager.')
  if (team.members.some((member) => !member.title.trim()))
    found.push('Give every member a title.')
  if (team.parent && !team.reports_to)
    found.push('Choose who the manager reports to in the team above.')
  return found
}
