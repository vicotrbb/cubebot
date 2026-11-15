import assert from 'node:assert/strict'
import test from 'node:test'
import { emojiMatches, symbol } from './emoji'

test('a search matches keyword prefixes and every word must match', () => {
  assert.ok(emojiMatches('rock').map(symbol).includes('🚀'))
  assert.deepEqual(emojiMatches('lock key').map(symbol), ['🔐'])
  assert.deepEqual(emojiMatches('zzzz'), [])
})
