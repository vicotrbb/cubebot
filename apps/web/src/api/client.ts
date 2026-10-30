import createClient, { type Client } from 'openapi-fetch'
import type { components, paths } from './schema'

export type Account = components['schemas']['Account']
export type Agent = components['schemas']['Agent']
export type Memory = components['schemas']['Memory']
export type Turn = components['schemas']['Turn']
export type TurnEvent = components['schemas']['Event']

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
/** Resolves to the response body, or throws ApiError for a failure status. */
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
const final = (turn: Turn) =>
  turn.status === 'complete' || turn.status === 'cancelled'
/** Applies turn snapshots, keeping history in sequence order. */
export function merge(turns: Turn[], incoming: Turn[]) {
  const byId = new Map(turns.map((turn) => [turn.id, turn]))
  for (const turn of incoming) {
    const known = byId.get(turn.id)
    // A finished turn never changes back; an older read must not undo it.
    if (!known || !final(known) || final(turn)) byId.set(turn.id, turn)
  }
  return [...byId.values()].sort((a, b) => a.sequence - b.sequence)
}
