import { useQuery, useQueryClient } from '@tanstack/react-query'
import { api, ok } from '../../api/client'
import type { Schemas } from '../../api/types'
import { keys } from '../../app/queries'
import { CALENDAR_KEY } from '../calendar/api'

export type Meeting = Schemas['Meeting']
export type MeetingSummary = Schemas['MeetingSummary']
export type EmailConnection = Schemas['EmailConnection']
export type Invite = Schemas['MeetingInvite']

export const meetingKey = (id: string) => [...keys.meetings, id]
const SETTINGS_KEY = ['meeting-settings']

export function useMeetings() {
  return useQuery({
    queryKey: keys.meetings,
    queryFn: () => ok(api.GET('/api/meetings')),
    refetchInterval: 30_000,
  })
}

export function useMeetingRecord(id: string) {
  return useQuery({
    queryKey: meetingKey(id),
    queryFn: () =>
      ok(api.GET('/api/meetings/{id}', { params: { path: { id } } })),
  })
}

export function useEmailConnections() {
  return useQuery({
    queryKey: [...keys.meetings, 'email-connections'],
    queryFn: () => ok(api.GET('/api/meetings/email-connections')),
    staleTime: 60_000,
  })
}

export function useMeetingSettings() {
  return useQuery({
    queryKey: SETTINGS_KEY,
    queryFn: () => ok(api.GET('/api/meetings/settings')),
    staleTime: 60_000,
  })
}

export function useRefresh() {
  const client = useQueryClient()
  return () =>
    Promise.all([
      client.invalidateQueries({ queryKey: keys.meetings }),
      client.invalidateQueries({ queryKey: CALENDAR_KEY }),
      client.invalidateQueries({ queryKey: SETTINGS_KEY }),
    ])
}

const MAILERS: Record<string, string> = {
  google: 'Gmail',
  microsoft: 'Outlook',
  email: 'Email',
}

export const connectionName = (connection: EmailConnection) =>
  [MAILERS[connection.connector] ?? connection.connector, connection.account]
    .filter(Boolean)
    .join(' · ')
