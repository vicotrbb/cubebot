import { useEffect, useState } from 'react'
import {
  api,
  failure,
  merge,
  ok,
  type Turn,
  type TurnEvent,
} from '../../api/client'
import { markRead } from '../../app/queries'

async function history(id: string) {
  const messages: Turn[] = []
  for (;;) {
    const page = await ok(
      api.GET('/api/conversations/{conversation}/messages', {
        params: {
          path: { conversation: id },
          query: { after: messages.at(-1)?.sequence ?? 0 },
        },
      }),
    )
    messages.push(...page)
    if (page.length < 100) return messages
  }
}

/** Follows one permanent conversation: its history, live turn snapshots and streamed text. */
export function useConversation(id: string) {
  const [turns, setTurns] = useState<Turn[]>([])
  const [streaming, setStreaming] = useState<Record<string, string>>({})
  const [loaded, setLoaded] = useState(false)
  const [readError, setReadError] = useState('')
  useEffect(() => {
    const source = new EventSource(`/api/conversations/${id}/events`)
    // History loads once the stream is following, so nothing falls between the two; a
    // reconnect reloads it to catch turns that changed without an event.
    source.onopen = () => {
      setReadError('')
      history(id).then(
        (loaded) => {
          setTurns((current) => merge(current, loaded))
          setLoaded(true)
        },
        (error) => setReadError(failure(error)),
      )
    }
    source.onerror = () =>
      setReadError(
        source.readyState === EventSource.CLOSED
          ? 'Live updates stopped. Reload the page to reconnect.'
          : 'Reconnecting to live updates…',
      )
    source.addEventListener('turn', (event) => {
      const turn: Turn = JSON.parse(event.data)
      setTurns((current) => merge(current, [turn]))
      // shortcut: a retried turn replayed on connect shows its interrupted run's text first;
      // record run boundaries on events if that confuses people.
      if (turn.status !== 'running')
        setStreaming((current) =>
          Object.fromEntries(
            Object.entries(current).filter(([key]) => key !== turn.id),
          ),
        )
    })
    source.addEventListener('turn_event', (event) => {
      const { turn_id, payload }: TurnEvent = JSON.parse(event.data)
      if (payload.kind === 'text_delta')
        setStreaming((current) => ({
          ...current,
          [turn_id]: (current[turn_id] ?? '') + payload.text,
        }))
    })
    return () => source.close()
  }, [id])
  const last = turns.at(-1)?.sequence ?? 0
  useEffect(() => {
    if (loaded && last > 0) void markRead('conversations', id, last)
  }, [id, loaded, last])
  const add = (turn: Turn) =>
    setTurns((current) =>
      current.some((known) => known.id === turn.id)
        ? current
        : merge(current, [turn]),
    )
  return { turns, streaming, loaded, readError, add }
}

export const OPEN = ['queued', 'running', 'delegated'] as const
export const UNSETTLED = ['queued', 'running', 'waiting', 'delegated'] as const
export const isOpen = (turn: Turn) =>
  (OPEN as readonly string[]).includes(turn.status)
export const isUnsettled = (turn: Turn) =>
  (UNSETTLED as readonly string[]).includes(turn.status)
