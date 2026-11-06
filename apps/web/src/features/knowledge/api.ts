import { useQuery, useQueryClient } from '@tanstack/react-query'
import { api, ApiError, ok } from '../../api/client'
import type { components } from '../../api/schema'
import type { Summary } from './tree'

export type Page = components['schemas']['Page']
export type Attachment = components['schemas']['Attachment']

const PAGE_POLL = 5_000
const TREE_POLL = 15_000

export const kbKeys = {
  tree: ['kb', 'tree'] as const,
  trash: ['kb', 'trash'] as const,
  page: (id: string) => ['kb', 'page', id] as const,
}

export function useTree() {
  return useQuery({
    queryKey: kbKeys.tree,
    queryFn: (): Promise<Summary[]> => ok(api.GET('/api/kb/pages')),
    refetchInterval: TREE_POLL,
  })
}

export function usePage(id: string) {
  return useQuery({
    queryKey: kbKeys.page(id),
    queryFn: () =>
      ok(api.GET('/api/kb/pages/{id}', { params: { path: { id } } })),
    refetchInterval: PAGE_POLL,
    staleTime: 0,
  })
}

export function useRefreshTree() {
  const client = useQueryClient()
  return () => client.invalidateQueries({ queryKey: kbKeys.tree })
}

export function createPage(kind: 'page' | 'folder', parent: string | null) {
  return ok(
    api.POST('/api/kb/pages', {
      body: {
        request_id: crypto.randomUUID(),
        kind,
        parent,
        title: '',
        body: '',
      },
    }),
  )
}

export function placePage(
  id: string,
  target: { parent: string | null; after: string | null },
) {
  return ok(
    api.POST('/api/kb/pages/{id}/place', {
      params: { path: { id } },
      body: target,
    }),
  )
}

export function trashPage(id: string) {
  return ok(api.DELETE('/api/kb/pages/{id}', { params: { path: { id } } }))
}

export class StaleSave extends Error {
  constructor(public revision: number) {
    super('The page changed since you opened it.')
  }
}

export async function savePage(
  id: string,
  change: {
    expected_revision?: number
    title?: string
    body?: string
    icon?: string
  },
) {
  const { data, error, response } = await api.PATCH('/api/kb/pages/{id}', {
    params: { path: { id } },
    body: change,
  })
  if (response.status === 409) {
    const refused = error as unknown as { revision?: number } | undefined
    throw new StaleSave(refused?.revision ?? 0)
  }
  if (!response.ok) throw new ApiError(response.status)
  return data as Page
}

export const attachmentUrl = (id: string) => `/api/kb/attachments/${id}`

export async function uploadAttachment(page: string, file: File) {
  const query = new URLSearchParams({
    name: file.name,
    request_id: crypto.randomUUID(),
  })
  const response = await fetch(`/api/kb/pages/${page}/attachments?${query}`, {
    method: 'POST',
    body: file,
    credentials: 'same-origin',
    headers: { 'content-type': file.type || 'application/octet-stream' },
  })
  if (response.status === 503)
    throw new Error('File storage is not configured on this installation.')
  if (response.status === 507)
    throw new Error('The installation is out of file storage.')
  if (response.status === 413)
    throw new Error(
      `${file.name} is larger than this installation’s upload limit.`,
    )
  if (!response.ok) throw new ApiError(response.status)
  return (await response.json()) as Attachment
}
