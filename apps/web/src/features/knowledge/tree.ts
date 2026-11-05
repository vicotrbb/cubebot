import type { components } from '../../api/schema'

export type Summary = components['schemas']['PageSummary']

export function childrenOf(pages: Summary[]) {
  const children = new Map<string, Summary[]>()
  for (const page of pages) {
    const parent = page.parent ?? ''
    children.set(parent, [...(children.get(parent) ?? []), page])
  }
  return children
}

export function pathTo(pages: Summary[], id: string) {
  const byId = new Map(pages.map((page) => [page.id, page]))
  const path: Summary[] = []
  let at = byId.get(id)
  while (at && !path.includes(at)) {
    path.unshift(at)
    at = at.parent ? byId.get(at.parent) : undefined
  }
  return path
}

export function under(pages: Summary[], id: string) {
  const children = childrenOf(pages)
  const found = new Set<string>()
  const walk = (at: string) => {
    for (const child of children.get(at) ?? []) {
      found.add(child.id)
      walk(child.id)
    }
  }
  walk(id)
  return found
}

export type Drop = 'before' | 'inside' | 'after'

export function dropTarget(
  pages: Summary[],
  moved: string,
  onto: Summary,
  drop: Drop,
): { parent: string | null; after: string | null } | null {
  if (moved === onto.id || under(pages, moved).has(onto.id)) return null
  if (drop === 'inside') {
    const last = childrenOf(pages)
      .get(onto.id)
      ?.filter((page) => page.id !== moved)
      .at(-1)
    return { parent: onto.id, after: last?.id ?? null }
  }
  const parent = onto.parent ?? null
  if (drop === 'after') return { parent, after: onto.id }
  const siblings = (childrenOf(pages).get(parent ?? '') ?? []).filter(
    (page) => page.id !== moved,
  )
  const before = siblings.findIndex((page) => page.id === onto.id)
  return { parent, after: siblings[before - 1]?.id ?? null }
}

export const titled = (page: { title: string; kind?: string }) =>
  page.title || (page.kind === 'folder' ? 'Untitled folder' : 'Untitled')
