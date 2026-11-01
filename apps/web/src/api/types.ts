import type { Avatar } from '../components/Cube'
import type { Agent, Room } from './client'

/* shortcut: optional fields bridge the console to the API contract in docs/design/console.md
   until `just api` regenerates schema.d.ts with them; remove once the schema carries them. */
export type AgentSummary = Agent & {
  avatar?: Avatar | null
  capabilities?: string[]
  desktop_active?: boolean
  pending_login?: boolean
  last_sequence?: number
  read_sequence?: number
}
export type RoomSummary = Room & {
  last_sequence?: number
  read_sequence?: number
}
export type Favorite = { kind: 'agent' | 'room'; id: string }
export type Preferences = {
  favorites: Favorite[]
  theme: 'system' | 'light' | 'dark'
  collapsed: string[]
}
export type Installation = {
  name: string
  terminal_enabled: boolean
  computer_use_enabled: boolean
  controller: 'kubernetes' | 'docker' | 'database_only'
}
export type ActivityItem = {
  kind:
    | 'login_pending'
    | 'turn_waiting'
    | 'turn_failed'
    | 'invitation_pending'
    | 'dream_published_skill'
  agent: string | null
  room: string | null
  turn: string | null
  at: string
  text: string
}
export type Session = {
  id: string
  administrator: boolean
  login?: string
  name?: string | null
}

export const CAPABILITIES = [
  'agent_manage',
  'conversation_read',
  'conversation_send',
  'memory_read',
  'memory_edit',
  'workspace_read',
  'workspace_write',
  'browser_view',
  'browser_control',
  'room_manage',
] as const
export type Capability = (typeof CAPABILITIES)[number]

/** Whether the viewer may do this on the agent; administrators may do everything. */
export function can(
  agent: AgentSummary,
  capability: Capability,
  session: Session,
) {
  if (session.administrator) return true
  // Without the contract's `capabilities`, the list only held managed agents.
  if (!agent.capabilities) return capability === 'agent_manage'
  return agent.capabilities.includes(capability)
}
