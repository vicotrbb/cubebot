/**
 * Holds an EventSource on `url` only while the page is visible. Over HTTP/1.1 a browser
 * keeps at most six connections per origin, so streams left open in background tabs would
 * queue every other request of the console. `setup` runs on each new source; its `onopen`
 * should reload whatever may have changed while the page was hidden. Returns the cleanup.
 */
export function followWhileVisible(
  url: string,
  setup: (source: EventSource) => void,
): () => void {
  let source: EventSource | undefined
  const sync = () => {
    if (document.hidden) {
      source?.close()
      source = undefined
    } else if (!source) {
      source = new EventSource(url)
      setup(source)
    }
  }
  sync()
  document.addEventListener('visibilitychange', sync)
  return () => {
    document.removeEventListener('visibilitychange', sync)
    source?.close()
  }
}
