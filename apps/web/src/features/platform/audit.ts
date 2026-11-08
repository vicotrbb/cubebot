export type Detail = Record<string, unknown>

export const FILTERS = [
  { label: 'All', areas: '' },
  { label: 'Sign-ins', areas: 'session' },
  { label: 'People and access', areas: 'account,invitation,team,access,oidc' },
  { label: 'Agents', areas: 'agent,broadcast,room' },
  { label: 'Desktop and vault', areas: 'desktop,login,vault' },
  { label: 'Connections', areas: 'provider,mcp' },
  { label: 'Installation', areas: 'installation,task' },
] as const

const text = (value: unknown) => (typeof value === 'string' ? value : '')

const planName = (provider: unknown) => {
  const name = text(provider)
  if (name === 'supergrok') return 'Grok'
  return name.startsWith('minimax') ? 'MiniMax' : 'ChatGPT'
}

const LIFECYCLE: Record<string, string> = {
  start: 'started',
  stop: 'stopped',
  restart: 'restarted',
  retire: 'retired',
}

const PHRASES: Record<string, (detail: Detail) => string> = {
  'account.bootstrap': () => 'set up the installation as',
  'account.create': () => 'created the account',
  'account.suspend': () => 'suspended',
  'account.avatar': (detail) =>
    detail.removed
      ? 'removed the profile picture of'
      : 'changed the profile picture of',
  'invitation.create': () => 'invited a new member',
  'invitation.revoke': () => 'revoked an invitation',
  'invitation.accept': () => 'accepted an invitation as',
  'team.create': () => 'created the group',
  'team.membership': () => 'changed the members of',
  'team.delete': () => 'deleted the group',
  'access.grant': (detail) => `granted ${capability(detail)} on`,
  'access.revoke': () => 'revoked a grant on',
  'oidc.revoke': () => 'revoked a single sign-on identity of',
  'oidc.configure': (detail) =>
    `set up single sign-on with ${text(detail.issuer) || 'an identity provider'}${detail.secret_changed ? ' and a new client secret' : ''}`,
  'oidc.disable': () => 'turned off single sign-on',
  'installation.settings': () =>
    'changed the installation’s name or storage limits',
  'oidc.link': () => 'linked a single sign-on identity to',
  'session.sign_in': (detail) =>
    detail.method === 'oidc'
      ? 'signed in with single sign-on as'
      : 'signed in as',
  'session.sign_out': () => 'signed out as',
  'session.sign_in_failed': (detail) =>
    `failed to sign in as ${text(detail.login) || 'an unknown login'}`,
  'agent.create': () => 'created the agent',
  'agent.update': () => 'changed the settings of',
  'agent.lifecycle': (detail) =>
    LIFECYCLE[text(detail.action)] ?? 'changed the state of',
  'agent.wallpaper': (detail) =>
    detail.reset ? 'reset the wallpaper of' : 'changed the wallpaper of',
  'vault.create': () => 'saved the vault item',
  'vault.update': () => 'changed the vault item',
  'vault.delete': () => 'deleted the vault item',
  'vault.fill': (detail) =>
    `filled the ${detail.field === 'username' ? 'username' : 'password'} of`,
  'mcp.create': () => 'added the MCP server',
  'mcp.update': () => 'changed the MCP server',
  'mcp.delete': () => 'removed the MCP server',
  'mcp.grant': (detail) =>
    detail.enabled
      ? 'gave an agent the MCP server'
      : 'took an agent off the MCP server',
  'provider.configure': (detail) =>
    `configured the AI provider: ${text(detail.provider)} · ${text(detail.model)}${detail.key_changed ? ' with a new key' : ''}`,
  'provider.sign_in': (detail) =>
    `signed in their ${planName(detail.provider)} plan`,
  'provider.sign_out': (detail) =>
    `signed out the ${planName(detail.provider)} plan${detail.revoked === false ? ', without the vendor confirming' : ''}`,
  'task.create': () => 'started a task',
  'task.stop': () => 'stopped a task',
  'task.settings': () => 'changed the task orchestrator’s settings',
  'room.create': () => 'created the room',
  'room.members': () => 'changed the members of',
  'desktop.take_control': (detail) =>
    detail.login
      ? `took control to log in to ${text(detail.login)} on`
      : 'took control of the desktop of',
  'desktop.hand_back': () => 'handed back the desktop of',
  'login.request': (detail) =>
    `asked a person to log in to ${text(detail.site)}`,
  'login.decline': () => 'declined the login request of',
  'broadcast.send': (detail) =>
    `broadcast to ${Number(detail.agents) || 0} ${Number(detail.agents) === 1 ? 'agent' : 'agents'}`,
}

const SELF: Record<string, (detail: Detail) => string> = {
  'account.bootstrap': () => 'set up the installation',
  'account.avatar': (detail) =>
    detail.removed
      ? 'removed their profile picture'
      : 'changed their profile picture',
  'invitation.accept': () => 'accepted an invitation',
  'session.sign_in': (detail) =>
    detail.method === 'oidc' ? 'signed in with single sign-on' : 'signed in',
  'session.sign_out': () => 'signed out',
}

function capability(detail: Detail) {
  return (
    text(detail.capability).replaceAll('_', ' ').replaceAll('.', ' ') ||
    'access'
  )
}

const COMPLETE = new Set([
  'invitation.create',
  'invitation.revoke',
  'session.sign_in_failed',
  'provider.configure',
  'provider.sign_in',
  'provider.sign_out',
  'login.request',
  'broadcast.send',
])

export function phrase(kind: string, detail: Detail, self = false) {
  const words = (self && SELF[kind]) || PHRASES[kind]
  return words?.(detail) ?? kind.replace('.', ' ').replaceAll('_', ' ')
}

export function isSelf(
  kind: string,
  actor?: string | null,
  resource?: string | null,
) {
  return Boolean(actor) && actor === resource && kind in SELF
}

export function target(
  kind: string,
  detail: Detail,
  name: string | null | undefined,
  resource: string | null | undefined,
) {
  if (COMPLETE.has(kind)) return null
  return (
    name || text(detail.name) || (resource ? `${resource.slice(0, 8)}…` : null)
  )
}

export function day(at: string, now = new Date()) {
  const date = new Date(at)
  const start = (value: Date) =>
    new Date(value.getFullYear(), value.getMonth(), value.getDate()).getTime()
  const days = Math.round((start(now) - start(date)) / 86_400_000)
  if (days === 0) return 'Today'
  if (days === 1) return 'Yesterday'
  return date.toLocaleDateString(undefined, {
    weekday: 'long',
    month: 'long',
    day: 'numeric',
    ...(date.getFullYear() === now.getFullYear() ? {} : { year: 'numeric' }),
  })
}
