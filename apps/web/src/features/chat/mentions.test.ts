import test from 'node:test'
import assert from 'node:assert/strict'
import { complete, mentionAt, suggest } from './mentions'

const members = [{ name: 'Ayla' }, { name: 'Test Agent' }, { name: 'Bob' }]
const names = (query: string) => suggest(members, query).map((m) => m.name)

test('a mention opens at an @ that starts a word, never inside an address', () => {
  assert.deepEqual(mentionAt('@', 1), { start: 0, query: '' })
  assert.deepEqual(mentionAt('hi @ay', 6), { start: 3, query: 'ay' })
  assert.deepEqual(mentionAt('(@Te', 4), { start: 1, query: 'Te' })
  assert.equal(mentionAt('mail bob@example.com', 20), null)
  assert.equal(mentionAt('mail bob@', 9), null)
  assert.equal(mentionAt('no mention', 10), null)
  assert.equal(mentionAt('@ay\nnext', 8), null)
  assert.equal(mentionAt('@ space', 7), null)
})

test('suggestions match the whole name or any word of it, whole names first', () => {
  assert.deepEqual(names(''), ['Ayla', 'Test Agent', 'Bob'])
  assert.deepEqual(names('a'), ['Ayla', 'Test Agent'])
  assert.deepEqual(names('test ag'), ['Test Agent'])
  assert.deepEqual(names('Ayla '), [])
})

test('completing replaces the typed mention and leaves the caret after it', () => {
  assert.deepEqual(
    complete('hi @te and more', 6, { start: 3, query: 'te' }, 'Test Agent'),
    { text: 'hi @Test Agent and more', caret: 15 },
  )
  assert.deepEqual(complete('@', 1, { start: 0, query: '' }, 'Bob'), {
    text: '@Bob ',
    caret: 5,
  })
})
