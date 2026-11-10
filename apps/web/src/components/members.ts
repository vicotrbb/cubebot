import type { AgentSummary } from '../api/types'

export type MemberOption = {
  kind: 'agent' | 'person'
  id: string
  name: string
  agent?: AgentSummary | undefined
}

export const memberKey = (member: MemberOption) => `${member.kind}:${member.id}`

export function matching(options: MemberOption[], query: string) {
  const words = query.toLowerCase().split(/\s+/).filter(Boolean)
  return options.filter((option) =>
    words.every((word) => option.name.toLowerCase().includes(word)),
  )
}

export function agentOptions(agents: AgentSummary[]): MemberOption[] {
  return agents
    .filter((agent) => agent.desired_state !== 'retired')
    .map((agent) => ({ kind: 'agent', id: agent.id, name: agent.name, agent }))
}

export function personOptions(
  people: { id: string; login: string; active: boolean }[],
): MemberOption[] {
  return people
    .filter((person) => person.active)
    .map((person) => ({ kind: 'person', id: person.id, name: person.login }))
}

export function without(options: MemberOption[], members: MemberOption[]) {
  const taken = new Set(members.map(memberKey))
  return options.filter((option) => !taken.has(memberKey(option)))
}
