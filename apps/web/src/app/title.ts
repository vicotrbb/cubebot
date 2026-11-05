const PAGES: [string, string][] = [
  ['/activity', 'Activity'],
  ['/agents/new', 'New agent'],
  ['/broadcast', 'Broadcast'],
  ['/files', 'Files'],
  ['/kb', 'Knowledge base'],
  ['/platform', 'Platform'],
  ['/rooms', 'Rooms'],
  ['/situation', 'Situation room'],
  ['/skills', 'Skills'],
  ['/tasks', 'Tasks'],
  ['/you', 'Profile'],
]

export function pageTitle(
  path: string,
  agents: readonly { id: string; name: string }[] = [],
): string {
  const page =
    PAGES.find(([prefix]) => path.startsWith(prefix))?.[1] ??
    agents.find((agent) => path.startsWith(`/agents/${agent.id}`))?.name
  return page ? `${page} · Cubebot` : 'Cubebot'
}
