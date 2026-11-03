import { useQueryClient } from '@tanstack/react-query'
import { useEffect } from 'react'
import type { AgentSummary, RoomSummary } from '../api/types'
import { keys } from './queries'

/**
 * Follows the installation stream (`GET /api/events`) and folds its frames into the
 * query cache, so the sidebar and the live preview update without polling each
 * conversation. A closed stream falls back to refreshing agents, rooms and the
 * session every 5 seconds.
 */
export function useInstallationStream(enabled: boolean) {
  const client = useQueryClient()
  useEffect(() => {
    if (!enabled) return
    let fallback: ReturnType<typeof setInterval> | undefined
    const source = new EventSource('/api/events')
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
      const bump: { id: string; last_sequence: number } = JSON.parse(event.data)
      client.setQueryData<RoomSummary[]>(keys.rooms, (current) =>
        current?.map((room) =>
          room.id === bump.id
            ? { ...room, last_sequence: bump.last_sequence }
            : room,
        ),
      )
    })
    source.onerror = () => {
      if (source.readyState !== EventSource.CLOSED || fallback) return
      // A closed stream (a lost session or a restart) falls back to polling.
      fallback = setInterval(() => {
        for (const queryKey of [keys.agents, keys.rooms, keys.session])
          void client.invalidateQueries({ queryKey })
      }, 5000)
    }
    return () => {
      source.close()
      clearInterval(fallback)
    }
  }, [enabled, client])
}
