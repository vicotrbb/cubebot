import { useQuery, useQueryClient } from '@tanstack/react-query'
import { api, ok } from '../../api/client'
import type { Schemas } from '../../api/types'
import { keys } from '../../app/queries'
import { answer, CONNECTIONS_KEY } from '../connections/api'

export type App = Schemas['App']
export type AppState = Schemas['AppState']

export function useApps(enabled = true) {
  return useQuery({
    queryKey: keys.apps,
    queryFn: () => ok(api.GET('/api/apps')),
    enabled,
    refetchInterval: 5_000,
  })
}

export function useConnectors(administrator: boolean) {
  const connections = useQuery({
    queryKey: CONNECTIONS_KEY,
    queryFn: () => answer(api.GET('/api/connections')),
    enabled: administrator,
    staleTime: 60_000,
  })
  return (slug: string) =>
    connections.data?.find((connection) => connection.slug === slug)
      ?.connector ?? slug
}

export function useAppActions() {
  const client = useQueryClient()
  const replace = (next: App) =>
    client.setQueryData<App[]>(keys.apps, (current) =>
      current?.map((app) => (app.slug === next.slug ? next : app)),
    )
  return {
    setDesired: async (slug: string, desired: 'running' | 'stopped') =>
      replace(
        await ok(
          api.POST('/api/apps/{slug}/state', {
            params: { path: { slug } },
            body: { desired },
          }),
        ),
      ),
    remove: async (slug: string) => {
      await ok(api.DELETE('/api/apps/{slug}', { params: { path: { slug } } }))
      client.setQueryData<App[]>(keys.apps, (current) =>
        current?.filter((app) => app.slug !== slug),
      )
    },
  }
}

export async function mintView(slug: string) {
  const view = await ok(
    api.POST('/api/apps/{slug}/views', { params: { path: { slug } } }),
  )
  return view.path
}

export async function probe(path: string) {
  const response = await fetch(path, {
    cache: 'no-store',
    credentials: 'omit',
  }).catch(() => null)
  void response?.body?.cancel()
  return response?.status ?? 502
}

export async function sendToAgent(
  agentId: string,
  message: string,
  app?: string,
) {
  await ok(
    api.POST('/api/conversations/{conversation}/messages', {
      params: { path: { conversation: agentId } },
      body: { request_id: crypto.randomUUID(), message, app: app ?? null },
    }),
  )
}

export type Opened = { src: string } | { problem: 'unavailable' | 'gone' }

const REMINT = [401, 403, 404]

export async function openView(slug: string, theme: string): Promise<Opened> {
  for (let attempt = 0; attempt < 2; attempt += 1) {
    const path = await mintView(slug).catch(() => null)
    if (!path) return { problem: 'gone' }
    const status = await probe(path)
    if (status === 502) return { problem: 'unavailable' }
    if (!REMINT.includes(status)) return { src: `${path}?theme=${theme}` }
  }
  return { problem: 'gone' }
}
