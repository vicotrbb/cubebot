import type { components } from '../../api/schema'

export type Grant = components['schemas']['GrantSummary']
export type Level = 'none' | 'view' | 'chat' | 'manage' | 'custom'
export type Target = { kind: 'agent' | 'room'; id: string }
type Needed = { scope: 'agent' | 'room' | 'conversation'; capability: string }

const AGENT: Record<Exclude<Level, 'none' | 'custom'>, Needed[]> = {
  view: [
    { scope: 'conversation', capability: 'conversation_read' },
    { scope: 'agent', capability: 'browser_view' },
    { scope: 'agent', capability: 'memory_read' },
    { scope: 'agent', capability: 'workspace_read' },
  ],
  chat: [],
  manage: [],
}
AGENT.chat = [
  ...AGENT.view,
  { scope: 'conversation', capability: 'conversation_send' },
]
AGENT.manage = [
  ...AGENT.chat,
  { scope: 'agent', capability: 'agent_manage' },
  { scope: 'agent', capability: 'browser_control' },
  { scope: 'agent', capability: 'memory_edit' },
  { scope: 'agent', capability: 'workspace_write' },
]
const ROOM: typeof AGENT = {
  view: [{ scope: 'room', capability: 'conversation_read' }],
  chat: [
    { scope: 'room', capability: 'conversation_read' },
    { scope: 'room', capability: 'conversation_send' },
  ],
  manage: [
    { scope: 'room', capability: 'conversation_read' },
    { scope: 'room', capability: 'conversation_send' },
    { scope: 'room', capability: 'room_manage' },
  ],
}

export const LEVELS: { id: Exclude<Level, 'custom'>; label: string }[] = [
  { id: 'none', label: 'No access' },
  { id: 'view', label: 'Can watch' },
  { id: 'chat', label: 'Can chat' },
  { id: 'manage', label: 'Can manage' },
]

export const LABEL: Record<Level, string> = {
  none: 'No access',
  view: 'Can watch',
  chat: 'Can chat',
  manage: 'Can manage',
  custom: 'Custom',
}

export const MEANING: Record<Exclude<Level, 'none' | 'custom'>, string> = {
  view: 'See the conversation, desktop, memory and files',
  chat: 'Also send messages',
  manage: 'Also change settings, take the desktop, edit memory and files',
}

const needs = (target: Target) => (target.kind === 'agent' ? AGENT : ROOM)

export function grantsOn(grants: Grant[], target: Target) {
  return grants.filter(
    (grant) =>
      grant.scope_id === target.id &&
      (target.kind === 'room'
        ? grant.scope_kind === 'room'
        : grant.scope_kind !== 'room'),
  )
}

const key = (scope: string, capability: string) => `${scope}:${capability}`

export function levelOf(onTarget: Grant[], target: Target): Level {
  if (onTarget.length === 0) return 'none'
  const held = new Set(onTarget.map((g) => key(g.scope_kind, g.capability)))
  for (const level of ['view', 'chat', 'manage'] as const) {
    const wanted = needs(target)[level]
    if (
      wanted.length === held.size &&
      wanted.every((need) => held.has(key(need.scope, need.capability)))
    )
      return level
  }
  return 'custom'
}

export function changesTo(
  onTarget: Grant[],
  target: Target,
  level: Exclude<Level, 'custom'>,
) {
  const wanted = level === 'none' ? [] : needs(target)[level]
  const keep = new Set(wanted.map((need) => key(need.scope, need.capability)))
  const held = new Set(onTarget.map((g) => key(g.scope_kind, g.capability)))
  return {
    revoke: onTarget.filter(
      (grant) => !keep.has(key(grant.scope_kind, grant.capability)),
    ),
    grant: wanted.filter((need) => !held.has(key(need.scope, need.capability))),
  }
}
