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
export async function request(path: string, body?: unknown): Promise<unknown> {
  const response = await fetch(`/api${path}`, {
    method: body === undefined ? 'GET' : 'POST',
    credentials: 'same-origin',
    cache: 'no-store',
    headers: body === undefined ? {} : { 'content-type': 'application/json' },
    body: body === undefined ? null : JSON.stringify(body),
  })
  if (!response.ok) throw new ApiError(response.status)
  return response.json()
}
export function failure(error: unknown) {
  return error instanceof Error
    ? error.message
    : 'The request failed. Retry shortly.'
}
function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}
function record(value: unknown): Record<string, unknown> {
  if (!isRecord(value)) throw new Error('Invalid server response.')
  return value
}
function string(value: unknown): string {
  if (typeof value !== 'string') throw new Error('Invalid server response.')
  return value
}
function number(value: unknown): number {
  if (typeof value !== 'number' || !Number.isSafeInteger(value))
    throw new Error('Invalid server response.')
  return value
}
function nullable(value: unknown): string | null {
  return value === null ? null : string(value)
}
export function array<T>(value: unknown, parse: (value: unknown) => T): T[] {
  if (!Array.isArray(value)) throw new Error('Invalid server response.')
  return value.map(parse)
}
export function account(value: unknown) {
  const data = record(value)
  if (typeof data.administrator !== 'boolean')
    throw new Error('Invalid server response.')
  return { id: string(data.id), administrator: data.administrator }
}
export type Account = ReturnType<typeof account>
export function agent(value: unknown) {
  const data = record(value)
  const idle = data.idle_policy
  if (idle !== 'sleep' && idle !== 'keep_running')
    throw new Error('Invalid server response.')
  return {
    id: string(data.id),
    name: string(data.name),
    instructions: string(data.instructions),
    revision: number(data.revision),
    idle_policy: idle,
  }
}
export type Agent = ReturnType<typeof agent>
export function turn(value: unknown) {
  const data = record(value)
  const status = string(data.status)
  if (
    !['queued', 'running', 'retrying', 'waiting', 'complete'].includes(status)
  )
    throw new Error('Invalid server response.')
  return {
    id: string(data.id),
    sequence: number(data.sequence),
    message: string(data.message),
    status,
    response: nullable(data.response),
    error: nullable(data.error),
    authority_id: nullable(data.authority_id),
  }
}
export type Turn = ReturnType<typeof turn>
export function provider(value: unknown) {
  if (value === null) return null
  const data = record(value)
  return { model: string(data.model), revision: number(data.revision) }
}

export async function pages<T extends { id: string }>(
  path: string,
  parse: (value: unknown) => T,
): Promise<T[]> {
  const items: T[] = []
  let after = ''
  for (;;) {
    const page = array(
      await request(`${path}${after ? `?after=${after}` : ''}`),
      parse,
    )
    items.push(...page)
    if (page.length < 100) return items
    const last = page.at(-1)
    if (!last) return items
    after = last.id
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
