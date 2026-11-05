const RETRY = 5000

/** Browsers keep six HTTP/1.1 connections per origin, so background-tab streams would queue every request; they also give up on a source after a failing status such as a 502, so a closed one is replaced after a wait. */
export function followWhileVisible(
  url: string | (() => string),
  setup: (source: EventSource) => void,
): () => void {
  let source: EventSource | undefined
  let retry: ReturnType<typeof setTimeout> | undefined
  const sync = () => {
    if (document.hidden) {
      source?.close()
      source = undefined
    } else if (!source) {
      const opened = new EventSource(typeof url === 'string' ? url : url())
      source = opened
      opened.addEventListener('error', () => {
        if (opened.readyState !== EventSource.CLOSED) return
        source = undefined
        clearTimeout(retry)
        retry = setTimeout(sync, RETRY)
      })
      setup(opened)
    }
  }
  sync()
  document.addEventListener('visibilitychange', sync)
  return () => {
    document.removeEventListener('visibilitychange', sync)
    clearTimeout(retry)
    source?.close()
  }
}
