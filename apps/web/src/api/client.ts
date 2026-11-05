import createClient, { type Client } from 'openapi-fetch'
import type { components, paths } from './schema'

type Agent = components['schemas']['Agent']
export type Memory = components['schemas']['Memory']
export type DreamRun = components['schemas']['DreamRun']
export type Skill = components['schemas']['Skill']
export type InstalledSkill = components['schemas']['InstalledSkill']
export type Turn = components['schemas']['Turn']
export type Room = components['schemas']['Room']
export type RoomMessage = components['schemas']['RoomMessage']
export type TurnEvent = components['schemas']['Event']
export type LoginHandoff = components['schemas']['LoginHandoff']
export type VaultItem = components['schemas']['VaultSummary']
export type Routine = components['schemas']['Routine']
export type Occurrence = components['schemas']['Occurrence']

export class ApiError extends Error {
  constructor(public status: number) {
    super(
      status === 401
        ? 'Your session ended. Sign in again.'
        : errorMessage(status),
    )
  }
}
function errorMessage(status: number) {
  if (status === 403 || status === 404)
    return 'This resource is unavailable or you do not have access.'
  if (status === 409)
    return 'This changed since you opened it. Refresh and try again.'
  if (status === 429) return 'Cubebot is busy. Wait a moment and retry.'
  if (status === 400) return 'Check the values and try again.'
  return 'Cubebot is unavailable. Your accepted messages remain saved. Retry shortly.'
}
export const api = createClient<paths>({
  credentials: 'same-origin',
  cache: 'no-store',
})
export async function ok<T>(
  result: Promise<{ data?: T; response: Response }>,
): Promise<T> {
  const { data, response } = await result
  if (!response.ok) throw new ApiError(response.status)
  return data as T
}
export function failure(error: unknown) {
  return error instanceof Error
    ? error.message
    : 'The request failed. Retry shortly.'
}
export async function agents(client: Client<paths> = api) {
  const items: Agent[] = []
  for (;;) {
    const last = items.at(-1)
    const page = await ok(
      client.GET('/api/agents', {
        params: { query: last ? { after: last.id } : {} },
      }),
    )
    items.push(...page)
    if (page.length < 100) return items
  }
}
export async function history<T extends { sequence: number }>(
  page: (after: number) => Promise<T[]>,
  start = 0,
) {
  const items: T[] = []
  for (;;) {
    const next = await page(items.at(-1)?.sequence ?? start)
    items.push(...next)
    if (next.length < 100) return items
  }
}
export function resumeAfter<T extends { sequence: number }>(
  items: T[],
  changing: (item: T) => boolean,
) {
  const oldest = items.find(changing)
  return oldest ? oldest.sequence - 1 : (items.at(-1)?.sequence ?? 0)
}
export function messageProblem(text: string): string | null {
  if (!text.trim()) return 'Write a message before sending.'
  if (new TextEncoder().encode(text).length > 16384)
    return 'Your message exceeds 16 KiB. Shorten it and try again.'
  if (text.includes('\0')) return 'Remove null characters and try again.'
  return null
}
export function rejectedInput(error: unknown): boolean {
  return (
    error instanceof ApiError && (error.status === 400 || error.status === 413)
  )
}
const final = (turn: { status: string }) =>
  turn.status === 'complete' ||
  turn.status === 'failed' ||
  turn.status === 'cancelled'
/** A finished turn still shows the work it sent off, which can outlive it. */
export const turnChanging = (turn: Turn) =>
  !final(turn) || turn.children.some((child) => !final(child))
export function merge(turns: Turn[], incoming: Turn[]) {
  const byId = new Map(turns.map((turn) => [turn.id, turn]))
  for (const turn of incoming) {
    const known = byId.get(turn.id)
    // A finished turn never changes back; an older read must not undo it.
    if (!known || !final(known) || final(turn)) byId.set(turn.id, turn)
  }
  return [...byId.values()].sort((a, b) => a.sequence - b.sequence)
}
