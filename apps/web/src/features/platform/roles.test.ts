import assert from 'node:assert/strict'
import { describe, test } from 'node:test'
import { heirs, lockOf, roleLockOf, roleSource, rolesFor } from './roles'

const owner = { id: 'o', owner: true }
const admin = { id: 'a', owner: false }
const person = (id: string, administrator = false, isOwner = false) => ({
  id,
  administrator,
  owner: isOwner,
})

describe('who may change whom', () => {
  test('nobody changes themselves or the owner', () => {
    assert.equal(lockOf(person('a', true), admin), 'You')
    assert.equal(lockOf(person('o', true, true), admin), 'Installation owner')
  })

  test('only the owner changes administrators', () => {
    assert.equal(
      lockOf(person('x', true), admin),
      'Only the owner changes administrators',
    )
    assert.equal(lockOf(person('x', true), owner), null)
    assert.equal(lockOf(person('m'), admin), null)
  })

  test('only the owner hands out the administrator role', () => {
    assert.ok(rolesFor(true).some((role) => role.id === 'administrator'))
    assert.ok(!rolesFor(false).some((role) => role.id === 'administrator'))
  })
})

describe('roles from groups', () => {
  const groups = [
    { name: 'Staff', active: true, members: ['m'], role: 'member' },
    { name: 'Audit', active: true, members: ['m'], role: 'auditor' },
    { name: 'Old', active: false, members: ['m'], role: 'administrator' },
    { name: 'Plain', active: true, members: ['p'], role: null },
  ]

  test('the strongest role among active groups wins', () => {
    assert.equal(roleSource('m', groups)?.name, 'Audit')
    assert.equal(roleSource('p', groups), null)
  })

  test('a role set by a group cannot be changed by hand', () => {
    assert.equal(roleLockOf(person('m'), owner, groups), 'Set by Audit')
    assert.equal(roleLockOf(person('p'), owner, groups), null)
  })
})

test('only active administrators and members can own things', () => {
  const people = [
    { id: '1', active: true, role: 'member' },
    { id: '2', active: true, role: 'guest' },
    { id: '3', active: false, role: 'administrator' },
    { id: '4', active: true, role: 'administrator' },
    { id: '5', active: true, role: 'auditor' },
  ]
  assert.deepEqual(
    heirs(people, '4').map((item) => item.id),
    ['1'],
  )
})
