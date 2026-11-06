import { ApiError } from '../../api/client'

export const CATALOG_KEY = ['connectors']
export const CONNECTIONS_KEY = ['connections']

/** The answer, or null for 202; a refusal throws with the server's own words when it gave them. */
export async function answer<T>(
  result: Promise<{ data?: T; error?: unknown; response: Response }>,
): Promise<T | null> {
  const { data, error, response } = await result
  if (response.status === 202) return null
  if (!response.ok) {
    const said = (error as { message?: unknown } | undefined)?.message
    if (typeof said === 'string' && said) throw new Error(sentence(said))
    throw new ApiError(response.status)
  }
  return data as T
}

function sentence(text: string) {
  const capital = text.charAt(0).toUpperCase() + text.slice(1)
  return /[.!?]$/.test(capital) ? capital : `${capital}.`
}
