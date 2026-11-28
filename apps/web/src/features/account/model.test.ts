import assert from 'node:assert/strict'
import { describe, test } from 'node:test'
import {
  fromUnix,
  groupedSecret,
  passwordProblem,
  recoveryFile,
  roleOf,
  signInRefusal,
} from './model'

describe('passwordProblem', () => {
  test('asks for twelve characters before checking the match', () => {
    assert.equal(
      passwordProblem('short', 'other'),
      'Use at least 12 characters.',
    )
  })
  test('names a mismatch', () => {
    assert.equal(
      passwordProblem('twelve chars!', 'twelve chars?'),
      'The two passwords do not match.',
    )
  })
  test('counts characters, not UTF-16 units', () => {
    assert.notEqual(passwordProblem('🔑'.repeat(6), '🔑'.repeat(6)), '')
    assert.equal(passwordProblem('🔑'.repeat(12), '🔑'.repeat(12)), '')
  })
})

describe('roleOf', () => {
  test('explains the limited roles', () => {
    assert.deepEqual(roleOf({ role: 'auditor', owner: false }), {
      label: 'Auditor',
      detail: 'Reads everything, changes nothing.',
    })
    assert.equal(roleOf({ role: 'guest', owner: false }).label, 'Guest')
  })
  test('names the installation owner', () => {
    assert.equal(
      roleOf({ role: 'administrator', owner: true }).label,
      'Installation owner',
    )
  })
})

test('groupedSecret splits in fours and drops spaces', () => {
  assert.equal(groupedSecret('JBSW Y3DPEHPK3PXPA'), 'JBSW Y3DP EHPK 3PXP A')
})

test('recoveryFile lists every code on its own line', () => {
  const file = recoveryFile(['ab12c-3de45', 'ff00a-11b22'], 'ana', 'Home')
  assert.ok(file.split('\n').includes('ab12c-3de45'))
  assert.ok(file.split('\n').includes('ff00a-11b22'))
  assert.equal(file.startsWith('Cubebot recovery codes for ana on Home'), true)
})

test('fromUnix keeps a missing time missing', () => {
  assert.equal(fromUnix(null), null)
  assert.equal(fromUnix(0), null)
  assert.equal(fromUnix(1_700_000_000), '2023-11-14T22:13:20.000Z')
})

describe('signInRefusal', () => {
  test('asks for a code only on two_factor_required', () => {
    assert.equal(signInRefusal(401, { error: 'two_factor_required' }), 'code')
    assert.equal(signInRefusal(401, { error: 'unauthorized' }), 'wrong')
    assert.equal(signInRefusal(401, undefined), 'wrong')
    assert.equal(signInRefusal(429, { error: 'two_factor_required' }), 'other')
  })
})
