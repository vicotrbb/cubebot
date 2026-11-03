import type { components } from './schema'

export type Schemas = components['schemas']
export type AgentSummary = Schemas['Agent']
export type RoomSummary = Schemas['Room']
export type Preferences = Required<Schemas['Preferences']>
export type Installation = Schemas['InstallationSummary']
export type Session = Schemas['Account']

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
  return session.administrator || agent.capabilities.includes(capability)
}
