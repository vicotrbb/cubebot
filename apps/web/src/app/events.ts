import { useQueryClient } from '@tanstack/react-query'
import { useEffect } from 'react'
import type { AgentSummary, RoomSummary } from '../api/types'
import { keys } from './queries'

/**
 * Follows the installation stream (`GET /api/events`) and folds its frames into the
 * query cache, so the sidebar and the live preview update without polling each
 * conversation. Without the route, the agent list refreshes every 5 seconds instead.
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
      // shortcut: a closed stream (404 before the contract lands, or a lost session)
      // falls back to polling; the stream is the intended path.
      fallback = setInterval(
        () => void client.invalidateQueries({ queryKey: keys.agents }),
        5000,
      )
    }
    return () => {
      source.close()
      clearInterval(fallback)
    }
  }, [enabled, client])
}
