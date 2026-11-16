import { useQuery } from '@tanstack/react-query'
import { api, ok } from '../../api/client'
import type { components } from '../../api/schema'

export type Reading = components['schemas']['Reading']
export type Usage = components['schemas']['Usage']

export const STALE_SECONDS = 15
export const BUSY = 0.9

export function useVitals(agent: string, enabled: boolean) {
  return useQuery({
    queryKey: ['agent-vitals', agent],
    queryFn: async () =>
      (await ok(
        api.GET('/api/agents/{id}/vitals', { params: { path: { id: agent } } }),
      )) ?? null,
    enabled,
    refetchInterval: 3_000,
  })
}

const UNITS = ['B', 'kB', 'MB', 'GB', 'TB'] as const

export function bytes(value: number) {
  const step = Math.min(
    UNITS.length - 1,
    value < 1000 ? 0 : Math.floor(Math.log10(value) / 3),
  )
  const scaled = value / 1000 ** step
  const digits = scaled < 10 && step > 0 ? 1 : 0
  const number = new Intl.NumberFormat(undefined, {
    maximumFractionDigits: digits,
  }).format(scaled)
  return `${number} ${UNITS[step]}`
}

export const rate = (value: number) => `${bytes(value)}/s`

export const share = (usage: Usage) =>
  usage.total > 0 ? Math.min(1, usage.used / usage.total) : 0

export const percent = (value: number) => `${Math.round(value * 100)}%`

export function age(seconds: number) {
  if (seconds < 5) return 'just now'
  if (seconds < 60) return `${seconds} s ago`
  const minutes = Math.round(seconds / 60)
  return minutes < 60
    ? `${minutes} min ago`
    : `${Math.round(minutes / 60)} h ago`
}
