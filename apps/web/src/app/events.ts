import { useQueryClient } from '@tanstack/react-query'
import { useEffect } from 'react'
import type { AgentSummary, RoomSummary } from '../api/types'
import { keys } from './queries'
import { followWhileVisible } from './stream'

/**
 * Follows the installation stream (`GET /api/events`) and folds its frames into the
 * query cache, so the sidebar and the live preview update without polling each
 * conversation. Each time the stream is refused or lost for good it refreshes agents,
 * rooms and the session, which is how a lost session is noticed. The stream pauses while
 * the tab is hidden; reopening it refreshes what changed meanwhile.
 */
export function useInstallationStream(enabled: boolean) {
  const client = useQueryClient()
  useEffect(() => {
    if (!enabled) return
    let opened = false
    const refresh = () => {
      for (const queryKey of [keys.agents, keys.rooms, keys.session])
        void client.invalidateQueries({ queryKey })
    }
    return followWhileVisible('/api/events', (source) => {
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
        if (source.readyState === EventSource.CLOSED) refresh()
      }
    })
  }, [enabled, client])
}
