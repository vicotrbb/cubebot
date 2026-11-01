import {
  QueryClient,
  useMutation,
  useQuery,
  useQueryClient,
} from '@tanstack/react-query'
import { agents as listAgents, api, ApiError, ok } from '../api/client'
import type {
  ActivityItem,
  AgentSummary,
  Installation,
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
}

/** A JSON GET outside the generated contract, or `fallback` when the route is absent. */
export async function fetchJson<T>(path: string, fallback?: T): Promise<T> {
  const response = await fetch(path, {
    credentials: 'same-origin',
    cache: 'no-store',
  })
  if (response.status === 404 && fallback !== undefined) return fallback
  if (!response.ok) throw new ApiError(response.status)
  return (await response.json()) as T
}

export async function sendJson<T>(
  path: string,
  method: 'POST' | 'PUT' | 'PATCH' | 'DELETE',
  body?: unknown,
): Promise<T> {
  const response = await fetch(path, {
    method,
    credentials: 'same-origin',
    headers: body === undefined ? {} : { 'content-type': 'application/json' },
    body: body === undefined ? null : JSON.stringify(body),
  })
  if (!response.ok) throw new ApiError(response.status)
  const text = await response.text()
  return (text ? JSON.parse(text) : null) as T
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
    queryFn: () =>
      fetchJson<Installation>('/api/installation', {
        name: 'Cubebot',
        terminal_enabled: true,
        computer_use_enabled: true,
        controller: 'database_only',
      }),
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
      fetchJson<Preferences>(
        '/api/identity/me/preferences',
        DEFAULT_PREFERENCES,
      ).then((loaded) => ({ ...DEFAULT_PREFERENCES, ...loaded })),
    staleTime: Infinity,
  })
}
export function useSavePreferences() {
  const client = useQueryClient()
  return useMutation({
    mutationFn: (next: Preferences) =>
      sendJson<Preferences | null>('/api/identity/me/preferences', 'PUT', next)
        .then(() => next)
        .catch((error) => {
          // shortcut: without the preferences route the choice lives in this tab only;
          // the bridge in api/types.ts names the contract.
          if (error instanceof ApiError && error.status === 404) return next
          throw error
        }),
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
    queryFn: () => fetchJson<ActivityItem[]>('/api/activity', []),
    refetchInterval: 30_000,
  })
}

/** Marks the conversation read up to `sequence`; ignored by servers without cursors. */
export async function markRead(
  scope: 'conversations' | 'rooms',
  id: string,
  sequence: number,
) {
  try {
    await sendJson(`/api/${scope}/${id}/read`, 'PUT', { sequence })
  } catch {
    /* A missing route or a lost request leaves the cursor where it was. */
  }
}

export function unread(item: {
  last_sequence?: number
  read_sequence?: number
}) {
  return (item.last_sequence ?? 0) > (item.read_sequence ?? 0)
}
