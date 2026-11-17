export type FrameTheme = 'light' | 'dark'

const DARK = '(prefers-color-scheme: dark)'

export function currentTheme(): FrameTheme {
  const chosen = document.documentElement.dataset.theme
  if (chosen === 'light' || chosen === 'dark') return chosen
  return matchMedia(DARK).matches ? 'dark' : 'light'
}

type Ask = { type: 'cubebot:ask'; id: unknown; text: unknown }

function isAsk(data: unknown): data is Ask {
  return (data as Ask | null)?.type === 'cubebot:ask'
}

function reason(error: unknown) {
  return error instanceof Error ? error.message : 'The message was not sent'
}

export function bridge(
  frame: HTMLIFrameElement,
  handlers: { ask(text: string): Promise<void>; expired(): void },
): () => void {
  const answered = new Set<unknown>()
  let sending = false
  let theme = currentTheme()
  const post = (message: object) =>
    frame.contentWindow?.postMessage(message, '*')
  const answer = (id: unknown, ok: boolean, error?: string) =>
    post({ type: 'cubebot:asked', id, ok, ...(error ? { error } : {}) })

  const ask = async ({ id, text }: Ask) => {
    if (answered.has(id)) return
    answered.add(id)
    if (typeof text !== 'string' || !text.trim())
      return answer(id, false, 'The message is empty')
    if (sending) return answer(id, false, 'Wait for the last message to send')
    sending = true
    try {
      await handlers.ask(text.trim())
      answer(id, true)
    } catch (error) {
      answer(id, false, reason(error))
    } finally {
      sending = false
    }
  }

  const receive = (event: MessageEvent) => {
    if (!frame.contentWindow || event.source !== frame.contentWindow) return
    const data: unknown = event.data
    if ((data as { type?: unknown } | null)?.type === 'cubebot:ready') {
      answered.clear()
      post({ type: 'cubebot:theme', theme })
    }
    if (isAsk(data)) void ask(data)
    if ((data as { type?: unknown } | null)?.type === 'cubebot:expired')
      handlers.expired()
  }

  const sync = () => {
    const next = currentTheme()
    if (next === theme) return
    theme = next
    post({ type: 'cubebot:theme', theme })
  }

  const scheme = matchMedia(DARK)
  const observer = new MutationObserver(sync)
  observer.observe(document.documentElement, {
    attributes: true,
    attributeFilter: ['data-theme'],
  })
  scheme.addEventListener('change', sync)
  window.addEventListener('message', receive)
  return () => {
    window.removeEventListener('message', receive)
    scheme.removeEventListener('change', sync)
    observer.disconnect()
  }
}
