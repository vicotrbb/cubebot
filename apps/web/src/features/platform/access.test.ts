// @ts-expect-error bun provides this at test time
import { expect, test } from 'bun:test'
import { changesTo, grantsOn, levelOf, type Grant } from './access'

const agent = { kind: 'agent', id: 'a' } as const
const room = { kind: 'room', id: 'a' } as const
const grant = (scope_kind: string, capability: string): Grant => ({
  id: `${scope_kind}${capability}`,
  revision: 1,
  account_id: 'p',
  team_id: null,
  scope_kind,
  scope_id: 'a',
  capability,
})
const watching = [
  grant('conversation', 'conversation_read'),
  grant('agent', 'browser_view'),
  grant('agent', 'memory_read'),
  grant('agent', 'workspace_read'),
]

test('levelOf names exact sets and calls anything else custom', () => {
  expect(levelOf([], agent)).toBe('none')
  expect(levelOf(watching, agent)).toBe('view')
  expect(
    levelOf([...watching, grant('conversation', 'conversation_send')], agent),
  ).toBe('chat')
  expect(levelOf([grant('agent', 'agent_manage')], agent)).toBe('custom')
  expect(levelOf([grant('room', 'conversation_read')], room)).toBe('view')
})

test('changesTo keeps what stays and touches only the difference', () => {
  const up = changesTo(watching, agent, 'chat')
  expect(up.revoke).toEqual([])
  expect(up.grant).toEqual([
    { scope: 'conversation', capability: 'conversation_send' },
  ])
  const off = changesTo(watching, agent, 'none')
  expect(off.revoke.length).toBe(4)
  expect(off.grant).toEqual([])
  const custom = [grant('agent', 'agent_manage')]
  expect(changesTo(custom, agent, 'view').revoke).toEqual(custom)
})

test('grantsOn tells a room from an agent with the same id', () => {
  const both = [...watching, grant('room', 'conversation_read')]
  expect(grantsOn(both, agent).length).toBe(4)
  expect(grantsOn(both, room).length).toBe(1)
})
