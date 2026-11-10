const UNITS: [number, string][] = [
  [86_400, 'day'],
  [3_600, 'hour'],
  [60, 'minute'],
]

export function uptime(seconds: number) {
  const parts: string[] = []
  let rest = Math.max(0, Math.floor(seconds))
  for (const [size, unit] of UNITS) {
    const count = Math.floor(rest / size)
    rest -= count * size
    if (count > 0 && parts.length < 2)
      parts.push(`${count} ${unit}${count === 1 ? '' : 's'}`)
  }
  return parts.length > 0 ? parts.join(', ') : 'under a minute'
}

export const DEPLOYMENTS: Record<string, [string, string]> = {
  kubernetes: ['Kubernetes', 'Each agent runs in its own pod'],
  docker: ['Docker', 'Each agent runs as a container on this host'],
  database_only: ['Database only', 'Agents are kept but never started'],
}

export function plural(count: number, one: string, many = `${one}s`) {
  return `${count} ${count === 1 ? one : many}`
}
