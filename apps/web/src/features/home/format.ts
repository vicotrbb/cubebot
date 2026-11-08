export function money(usd: number) {
  if (usd === 0) return '$0.00'
  if (usd < 0.01) return '<$0.01'
  if (usd < 1000) return `$${usd.toFixed(2)}`
  return `$${Math.round(usd).toLocaleString('en-US')}`
}

export function count(tokens: number) {
  if (tokens < 1000) return String(tokens)
  if (tokens < 1_000_000) return `${Number((tokens / 1000).toFixed(1))}K`
  return `${Number((tokens / 1_000_000).toFixed(1))}M`
}

type Pace = 'none' | 'fine' | 'near' | 'over'

export function pace(spent: number, budget: number | null | undefined): Pace {
  if (!budget) return 'none'
  if (spent >= budget) return 'over'
  return spent >= budget * 0.8 ? 'near' : 'fine'
}

type Summary = {
  activity: string
  pending_login: boolean
}
type Facts = {
  agents: Summary[]
  work: { status: string }[]
  failedToday: number
  overBudget: number
}

export function headline({ agents, work, failedToday, overBudget }: Facts) {
  const waiting = work.filter((item) => item.status === 'waiting').length
  const logins = agents.filter((agent) => agent.pending_login).length
  const needs = [
    plural(waiting, 'turn is waiting', 'turns are waiting'),
    plural(logins, 'login is waiting', 'logins are waiting'),
    plural(failedToday, 'turn failed today', 'turns failed today'),
    plural(overBudget, 'agent is over budget', 'agents are over budget'),
  ].filter(Boolean)
  const working = agents.filter((agent) => agent.activity === 'working').length
  const state =
    working > 0
      ? `${working} of ${agents.length} ${agents.length === 1 ? 'agent' : 'agents'} working.`
      : `${agents.length} ${agents.length === 1 ? 'agent' : 'agents'}, none working.`
  if (needs.length === 0)
    return { calm: true, text: 'Nothing needs you.', state }
  const [first = '', ...rest] = needs
  const joined = [first.charAt(0).toUpperCase() + first.slice(1), ...rest].join(
    ', ',
  )
  return { calm: false, text: `${joined}.`, state }
}

function plural(amount: number, one: string, many: string) {
  if (amount === 0) return ''
  return amount === 1 ? `1 ${one}` : `${amount} ${many}`
}

export function fillDays<T extends { day: string }>(
  days: T[],
  empty: (day: string) => T,
  today: Date,
  span = 30,
): T[] {
  const known = new Map(days.map((day) => [day.day, day]))
  return Array.from({ length: span }, (_, index) => {
    const date = new Date(today)
    date.setUTCDate(date.getUTCDate() - (span - 1 - index))
    const day = date.toISOString().slice(0, 10)
    return known.get(day) ?? empty(day)
  })
}
