import assert from 'node:assert/strict'
import test from 'node:test'
import { age, bytes, rate, share } from './reading'

test('sizes read in decimal units with one decimal under ten', () => {
  assert.equal(bytes(512), '512 B')
  assert.equal(bytes(1_500), '1.5 kB')
  assert.equal(bytes(23_400_000), '23 MB')
  assert.equal(bytes(2_147_483_648), '2.1 GB')
  assert.equal(rate(0), '0 B/s')
})

test('a share never exceeds the whole and an unknown total is empty', () => {
  assert.equal(share({ used: 3, total: 4 }), 0.75)
  assert.equal(share({ used: 9, total: 4 }), 1)
  assert.equal(share({ used: 9, total: 0 }), 0)
})

test('the reading age stays short', () => {
  assert.equal(age(2), 'just now')
  assert.equal(age(42), '42 s ago')
  assert.equal(age(150), '3 min ago')
  assert.equal(age(7_200), '2 h ago')
})
