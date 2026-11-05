import { useEffect, useRef, useState } from 'react'
import {
  api,
  failure,
  history,
  merge,
  ok,
  resumeAfter,
  turnChanging,
  type Turn,
  type TurnEvent,
} from '../../api/client'
import { markRead } from '../../app/queries'
import { followWhileVisible } from '../../app/stream'

export function useConversation(id: string) {
  const [turns, setTurns] = useState<Turn[]>([])
  const [streaming, setStreaming] = useState<Record<string, string>>({})
  const [loaded, setLoaded] = useState(false)
  const [readError, setReadError] = useState('')
  const known = useRef(turns)
  useEffect(() => {
    known.current = turns
  }, [turns])
  useEffect(() => {
    let loading: AbortController | undefined
    // What the stream delivers first is no whole history, so only a finished load can resume.
    let whole = false
    const stop = followWhileVisible(
      `/api/conversations/${id}/events`,
      (source) => {
        // A new source replays a running turn from its first event.
        setStreaming({})
        // History loads once the stream is following, so nothing falls between the two.
        source.onopen = () => {
          setReadError('')
          loading?.abort()
          const { signal } = (loading = new AbortController())
          history(
            (after) =>
              ok(
                api.GET('/api/conversations/{conversation}/messages', {
                  signal,
                  params: { path: { conversation: id }, query: { after } },
                }),
              ),
            whole ? resumeAfter(known.current, turnChanging) : 0,
          ).then(
            (loaded) => {
              if (signal.aborted) return
              whole = true
              setTurns((current) => merge(current, loaded))
              setLoaded(true)
            },
            (error) => {
              if (!signal.aborted) setReadError(failure(error))
            },
          )
        }
        source.onerror = () => setReadError('Reconnecting to live updates…')
        source.addEventListener('turn', (event) => {
          const turn: Turn = JSON.parse(event.data)
          setTurns((current) => merge(current, [turn]))
          // shortcut: a retried turn replayed on connect shows its interrupted run's text first; record run boundaries on events if that confuses people.
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
      },
    )
    return () => {
      stop()
      loading?.abort()
    }
  }, [id])
  // A call's own sequence is older than the turns it holds, which are what unread counts.
  const final = turns.at(-1)
  const last = final?.call?.turns.at(-1)?.sequence ?? final?.sequence ?? 0
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

const OPEN = ['queued', 'running', 'delegated'] as const
const UNSETTLED = ['queued', 'running', 'waiting', 'delegated'] as const
export const isOpen = (turn: Turn) =>
  (OPEN as readonly string[]).includes(turn.status)
export const isUnsettled = (turn: Turn) =>
  (UNSETTLED as readonly string[]).includes(turn.status)
