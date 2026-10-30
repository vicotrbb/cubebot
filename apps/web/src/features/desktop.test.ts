import test from 'node:test'
import assert from 'node:assert/strict'
import { keyAction } from './desktop'

const plain = { ctrlKey: false, altKey: false, metaKey: false, shiftKey: false }

test('characters are typed and other keys go with their modifiers', () => {
  assert.deepEqual(keyAction({ ...plain, key: 'A', shiftKey: true }), {
    action: 'type',
    text: 'A',
  })
  assert.deepEqual(keyAction({ ...plain, key: 'Enter' }), {
    action: 'key',
    keys: ['enter'],
  })
  assert.deepEqual(
    keyAction({ ...plain, key: 'L', ctrlKey: true, shiftKey: true }),
    { action: 'key', keys: ['ctrl', 'shift', 'l'] },
  )
  assert.deepEqual(keyAction({ ...plain, key: ' ', ctrlKey: true }), {
    action: 'key',
    keys: ['ctrl', 'space'],
  })
  assert.equal(keyAction({ ...plain, key: 'Shift', shiftKey: true }), null)
})
