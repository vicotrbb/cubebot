import {
  QueryClient,
  useMutation,
  useQuery,
  useQueryClient,
} from '@tanstack/react-query'
import type { components } from '../api/schema'
import type { Client } from 'openapi-fetch'
import { agents as listAgents, api, ApiError, ok } from '../api/client'
import type { paths } from '../api/schema'
import type {
  AgentSummary,
  Preferences,
  RoomSummary,
  Session,
} from '../api/types'

export const queryClient = new QueryClient({
  defaultOptions: {
    queries: { retry: false, staleTime: 10_000, refetchOnWindowFocus: false },
  },
})

export const keys = {
  session: ['session'] as const,
  installation: ['installation'] as const,
  preferences: ['preferences'] as const,
  agents: ['agents'] as const,
  rooms: ['rooms'] as const,
  activity: ['activity'] as const,
  avatars: ['avatars'] as const,
  broadcasts: ['broadcasts'] as const,
}

export function useSession() {
  return useQuery({
    queryKey: keys.session,
    queryFn: async (): Promise<Session | null> => {
      try {
        return await ok(api.GET('/api/identity/me'))
      } catch (error) {
        if (error instanceof ApiError && error.status === 401) return null
        throw error
      }
    },
    staleTime: 60_000,
  })
}

export function useInstallation() {
  return useQuery({
    queryKey: keys.installation,
    queryFn: () => ok(api.GET('/api/installation')),
    staleTime: Infinity,
  })
}

const DEFAULT_PREFERENCES: Preferences = {
  favorites: [],
  theme: 'system',
  collapsed: [],
}
export function usePreferences() {
  return useQuery({
    queryKey: keys.preferences,
    queryFn: () =>
      ok(api.GET('/api/identity/me/preferences')).then(
        (loaded): Preferences => ({ ...DEFAULT_PREFERENCES, ...loaded }),
      ),
    staleTime: Infinity,
  })
}
export function useSavePreferences() {
  const client = useQueryClient()
  return useMutation({
    mutationFn: (next: Preferences) =>
      ok(api.PUT('/api/identity/me/preferences', { body: next })).then(
        () => next,
      ),
    onMutate: (next) => client.setQueryData(keys.preferences, next),
  })
}

export function useAgents() {
  return useQuery({
    queryKey: keys.agents,
    queryFn: (): Promise<AgentSummary[]> => listAgents(),
  })
}
export function useAgent(id: string) {
  const list = useAgents()
  return { ...list, agent: list.data?.find((agent) => agent.id === id) }
}

export function useRooms() {
  return useQuery({
    queryKey: keys.rooms,
    queryFn: async (): Promise<RoomSummary[]> => ok(api.GET('/api/rooms')),
  })
}

export function useActivity() {
  return useQuery({
    queryKey: keys.activity,
    queryFn: () => ok(api.GET('/api/activity')),
    refetchInterval: 30_000,
  })
}

/** How many inbox items arrived since the person last opened it. */
export function useUnreadActivity() {
  const activity = useActivity()
  return activity.data?.filter((item) => item.unread).length ?? 0
}

/** Marks the inbox seen on the server, so the badge clears on every device, and here at once. */
export async function markActivityRead() {
  queryClient.setQueryData<components['schemas']['ActivityItem'][]>(
    keys.activity,
    (current) => current?.map((item) => ({ ...item, unread: false })),
  )
  await api.PUT('/api/activity/read').catch(() => undefined)
}

export function useSituation() {
  return useQuery({
    queryKey: ['situation'],
    queryFn: () => ok(api.GET('/api/situation')),
    refetchInterval: 10_000,
  })
}

/** People's profile picture revisions; a new picture is a new URL. */
export function useAvatars() {
  return useQuery({
    queryKey: keys.avatars,
    queryFn: () => ok(api.GET('/api/identity/avatars')),
    staleTime: 5 * 60_000,
  })
}

/** The Broadcast channel, for administrators: the newest 100 broadcasts and the read cursor. */
export function useBroadcasts(enabled: boolean, live = false) {
  return useQuery({
    queryKey: keys.broadcasts,
    queryFn: () =>
      ok(api.GET('/api/broadcasts', { params: { query: { after: 0 } } })),
    enabled,
    // Deliveries change as agents answer; the stream only says a new broadcast arrived.
    refetchInterval: live ? 3000 : false,
  })
}

/** Marks the conversation read up to `sequence` and clears its unread mark in the cached list. */
export async function markRead(
  scope: 'conversations' | 'rooms',
  id: string,
  sequence: number,
  client: Client<paths> = api,
) {
  const request =
    scope === 'rooms'
      ? client.PUT('/api/rooms/{room}/read', {
          params: { path: { room: id } },
          body: { sequence },
        })
      : client.PUT('/api/conversations/{conversation}/read', {
          params: { path: { conversation: id } },
          body: { sequence },
        })
  const { response } = await request.catch(() => ({ response: null }))
  // A lost request leaves the cursor, and the mark, where they were.
  if (!response?.ok) return
  queryClient.setQueryData<{ id: string; read_sequence: number }[]>(
    scope === 'rooms' ? keys.rooms : keys.agents,
    (current) =>
      current?.map((item) =>
        item.id === id
          ? { ...item, read_sequence: Math.max(item.read_sequence, sequence) }
          : item,
      ),
  )
}

/** Ends the session and forgets everything cached for it. */
export async function signOut() {
  try {
    const { response } = await api.POST('/api/identity/logout')
    // 401: the session had already ended, which is what signing out wants.
    if (!response.ok && response.status !== 401)
      throw new ApiError(response.status)
  } finally {
    queryClient.setQueryData(keys.session, null)
    queryClient.clear()
  }
}

export function unread(item: { last_sequence: number; read_sequence: number }) {
  return item.last_sequence > item.read_sequence
}
