const PAGES: [string, string][] = [
  ['/activity', 'Activity'],
  ['/agents/new', 'New agent'],
  ['/rooms/new', 'New room'],
  ['/platform/general', 'General'],
  ['/platform/people', 'People'],
  ['/platform/groups', 'Groups and access'],
  ['/platform/sign-in', 'Sign-in'],
  ['/platform/providers', 'AI providers'],
  ['/platform/speech', 'Speech'],
  ['/platform/tasks', 'Task settings'],
  ['/platform/mcp', 'MCP servers'],
  ['/platform/vault', 'Vault'],
  ['/platform/audit', 'Audit log'],
  ['/platform/health', 'Health'],
  ['/broadcast', 'Broadcast'],
  ['/files', 'Files'],
  ['/kb', 'Knowledge base'],
  ['/platform', 'Platform'],
  ['/rooms', 'Rooms'],
  ['/teams/new', 'New team'],
  ['/teams', 'Team'],
  ['/menu', 'Browse'],
  ['/skills', 'Skills'],
  ['/tasks', 'Tasks'],
  ['/you', 'Profile'],
]

export function pageTitle(
  path: string,
  agents: readonly { id: string; name: string }[] = [],
  rooms: readonly { id: string; name: string }[] = [],
): string {
  const page =
    (path === '/' ? 'Home' : undefined) ??
    rooms.find((room) => path.startsWith(`/rooms/${room.id}`))?.name ??
    PAGES.find(([prefix]) => path.startsWith(prefix))?.[1] ??
    agents.find((agent) => path.startsWith(`/agents/${agent.id}`))?.name
  return page ? `${page} · Cubebot` : 'Cubebot'
}
