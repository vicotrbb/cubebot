import test from 'node:test'
import assert from 'node:assert/strict'
import { bytesFrom, unitsOf } from './limits'

const GIB = 1024 ** 3
const MIB = 1024 ** 2

test('an empty entry keeps the deployment value', () => {
  assert.equal(bytesFrom('', GIB), null)
  assert.equal(bytesFrom('   ', GIB), null)
  assert.equal(bytesFrom(null, GIB), null)
  assert.equal(unitsOf(null, GIB), '')
})

test('entries convert to whole bytes and back', () => {
  assert.equal(bytesFrom('9', GIB), 9 * GIB)
  assert.equal(bytesFrom('0.5', GIB), 512 * MIB)
  assert.equal(bytesFrom('25', MIB), 25 * MIB)
  assert.equal(unitsOf(25 * MIB, MIB), '25')
  assert.equal(unitsOf(9 * GIB + 123, GIB), '9')
  assert.equal(unitsOf(1.5 * GIB, GIB), '1.5')
})

test('a non-number is not sent as a value', () => {
  assert.equal(bytesFrom('lots', GIB), null)
})
