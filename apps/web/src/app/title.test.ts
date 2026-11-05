// @ts-expect-error bun provides this at test time
import { expect, test } from 'bun:test'
import { pageTitle } from './title'

const agents = [{ id: 'a1', name: 'Ayla' }]

test('names the page, an agent by name, and falls back to the product', () => {
  expect(pageTitle('/tasks/x', agents)).toBe('Tasks · Cubebot')
  expect(pageTitle('/agents/a1/status', agents)).toBe('Ayla · Cubebot')
  expect(pageTitle('/agents/new', agents)).toBe('New agent · Cubebot')
  expect(pageTitle('/agents/gone', agents)).toBe('Cubebot')
  expect(pageTitle('/')).toBe('Cubebot')
  expect(pageTitle('/platform/vault')).toBe('Vault · Cubebot')
  expect(pageTitle('/rooms/new')).toBe('New room · Cubebot')
  expect(pageTitle('/rooms/r1', agents, [{ id: 'r1', name: 'Launch' }])).toBe(
    'Launch · Cubebot',
  )
})
