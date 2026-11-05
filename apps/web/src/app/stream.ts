const RETRY = 5000

/**
 * Holds an EventSource on `url` only while the page is visible. Over HTTP/1.1 a browser
 * keeps at most six connections per origin, so streams left open in background tabs would
 * queue every other request of the console. `setup` runs on each new source; its `onopen`
 * should reload whatever may have changed while the page was hidden. A browser gives up on
 * a source after a failing status such as a 502, so a closed one is replaced after a wait.
 * Returns the cleanup.
 */
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
