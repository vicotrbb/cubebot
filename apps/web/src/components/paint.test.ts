import test from 'node:test'
import assert from 'node:assert/strict'
import { isDark } from './paint'

test('dark picks get paper eyes, light and mid picks keep ink', () => {
  assert.equal(isDark('#000000'), true)
  assert.equal(isDark('#4d4642'), true)
  assert.equal(isDark('#ffffff'), false)
  assert.equal(isDark('#f26a1b'), false)
  assert.equal(isDark('#4f7fb8'), false)
})
