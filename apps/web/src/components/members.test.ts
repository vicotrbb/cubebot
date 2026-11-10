import test from 'node:test'
import assert from 'node:assert/strict'
import type { AgentSummary } from '../api/types'
import { agentOptions, matching, personOptions, without } from './members'

const agent = (id: string, name: string, desired_state = 'running') =>
  ({ id, name, desired_state }) as AgentSummary

test('a retired agent is never offered', () => {
  const offered = agentOptions([
    agent('a', 'Ayla'),
    agent('b', 'Old', 'retired'),
  ])
  assert.deepEqual(
    offered.map((option) => option.id),
    ['a'],
  )
})

test('a suspended person is never offered', () => {
  const offered = personOptions([
    { id: 'p', login: 'marina', active: true },
    { id: 'q', login: 'gone', active: false },
  ])
  assert.deepEqual(
    offered.map((option) => option.name),
    ['marina'],
  )
})

test('search matches every word anywhere in the name, ignoring case', () => {
  const options = [
    ...agentOptions([agent('a', 'Ayla Finance'), agent('o', 'Orion')]),
    ...personOptions([{ id: 'p', login: 'theo.k', active: true }]),
  ]
  assert.deepEqual(
    matching(options, 'fin ay').map((option) => option.id),
    ['a'],
  )
  assert.equal(matching(options, '').length, 3)
  assert.deepEqual(
    matching(options, 'THEO').map((option) => option.kind),
    ['person'],
  )
})

test('members are not offered again, matched by kind and id', () => {
  const options = [
    ...agentOptions([agent('x', 'Ayla')]),
    ...personOptions([{ id: 'x', login: 'xavier', active: true }]),
  ]
  const left = without(options, options.slice(0, 1))
  assert.deepEqual(
    left.map((option) => option.kind),
    ['person'],
  )
})
