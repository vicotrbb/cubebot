const KEY = 'cubebot-stars'
const HOUR = 3_600_000

function compact(count: number): string {
  return count < 1000
    ? String(count)
    : `${(count / 1000).toFixed(1).replace(/\.0$/, '')}k`
}

function show(count: number): void {
  document.querySelectorAll<HTMLElement>('[data-stars]').forEach((node) => {
    node.textContent = compact(count)
  })
}

function cached(): number | null {
  try {
    const raw = sessionStorage.getItem(KEY)
    if (!raw) return null
    const { count, at } = JSON.parse(raw) as { count: number; at: number }
    return Date.now() - at < HOUR ? count : null
  } catch {
    return null
  }
}

async function refresh(): Promise<void> {
  const known = cached()
  if (known !== null) return show(known)
  try {
    const response = await fetch(
      'https://api.github.com/repos/vicotrbb/cubebot',
    )
    if (!response.ok) return
    const { stargazers_count: count } = (await response.json()) as {
      stargazers_count: number
    }
    show(count)
    sessionStorage.setItem(KEY, JSON.stringify({ count, at: Date.now() }))
  } catch {
    // The build-time count stays.
  }
}

void refresh()
