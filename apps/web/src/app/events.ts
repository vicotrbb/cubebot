import { useQueryClient } from '@tanstack/react-query'
import { useEffect } from 'react'
import type { AgentSummary, RoomSummary } from '../api/types'
import { keys } from './queries'
import { followWhileVisible } from './stream'

/**
 * Follows the installation stream (`GET /api/events`) and folds its frames into the
 * query cache, so the sidebar and the live preview update without polling each
 * conversation. A closed stream falls back to refreshing agents, rooms and the
 * session every 5 seconds. The stream pauses while the tab is hidden; reopening it
 * refreshes what changed meanwhile.
 */
export function useInstallationStream(enabled: boolean) {
  const client = useQueryClient()
  useEffect(() => {
    if (!enabled) return
    let fallback: ReturnType<typeof setInterval> | undefined
    let opened = false
    const refresh = () => {
      for (const queryKey of [keys.agents, keys.rooms, keys.session])
        void client.invalidateQueries({ queryKey })
    }
    const stop = followWhileVisible('/api/events', (source) => {
      source.onopen = () => {
        if (opened) refresh()
        opened = true
      }
      source.addEventListener('agent', (event) => {
        const agent: AgentSummary = JSON.parse(event.data)
        client.setQueryData<AgentSummary[]>(keys.agents, (current) => {
          if (!current) return current
          const known = current.some((item) => item.id === agent.id)
          return known
            ? current.map((item) => (item.id === agent.id ? agent : item))
            : [...current, agent]
        })
      })
      source.addEventListener('room', (event) => {
        const bump: { id: string; last_sequence: number } = JSON.parse(
          event.data,
        )
        client.setQueryData<RoomSummary[]>(keys.rooms, (current) =>
          current?.map((room) =>
            room.id === bump.id
              ? { ...room, last_sequence: bump.last_sequence }
              : room,
          ),
        )
      })
      // Only administrators get these; the feed itself carries the deliveries.
      source.addEventListener('broadcast', () => {
        void client.invalidateQueries({ queryKey: keys.broadcasts })
      })
      source.onerror = () => {
        if (source.readyState !== EventSource.CLOSED || fallback) return
        // A closed stream (a lost session or a restart) falls back to polling.
        fallback = setInterval(refresh, 5000)
      }
    })
    return () => {
      stop()
      clearInterval(fallback)
    }
  }, [enabled, client])
}
