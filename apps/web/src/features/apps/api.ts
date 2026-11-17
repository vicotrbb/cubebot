import { useQuery, useQueryClient } from '@tanstack/react-query'
import { api, ok } from '../../api/client'
import type { Schemas } from '../../api/types'
import { keys } from '../../app/queries'

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

const REFUSALS = ['unauthorized', 'forbidden', 'not_found']

async function refused(response: Response) {
  if (![401, 403, 404].includes(response.status)) return false
  if (!response.headers.get('content-type')?.includes('application/json'))
    return false
  const body = (await response.json().catch(() => null)) as {
    error?: unknown
  } | null
  return REFUSALS.includes(String(body?.error))
}

export async function probe(path: string) {
  const response = await fetch(path, {
    cache: 'no-store',
    credentials: 'omit',
  }).catch(() => null)
  if (!response) return 'unavailable'
  if (await refused(response)) return 'refused'
  void response.body?.cancel()
  return response.status === 502 ? 'unavailable' : 'open'
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

export async function openView(slug: string, theme: string): Promise<Opened> {
  for (let attempt = 0; attempt < 2; attempt += 1) {
    const path = await mintView(slug).catch(() => null)
    if (!path) return { problem: 'gone' }
    const probed = await probe(path)
    if (probed === 'unavailable') return { problem: 'unavailable' }
    if (probed === 'open') return { src: `${path}?theme=${theme}` }
  }
  return { problem: 'gone' }
}
