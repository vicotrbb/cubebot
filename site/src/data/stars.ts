// Read once per build; the browser refreshes it (scripts/stars.ts).
async function read(): Promise<number | null> {
  try {
    const response = await fetch(
      'https://api.github.com/repos/vicotrbb/cubebot',
      {
        headers: { accept: 'application/vnd.github+json' },
        signal: AbortSignal.timeout(4000),
      },
    )
    if (!response.ok) return null
    const body = (await response.json()) as { stargazers_count?: number }
    return body.stargazers_count ?? null
  } catch {
    return null
  }
}

export const stars = await read()

export function compact(count: number): string {
  return count < 1000
    ? String(count)
    : `${(count / 1000).toFixed(1).replace(/\.0$/, '')}k`
}
