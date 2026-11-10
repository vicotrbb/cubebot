import test from 'node:test'
import assert from 'node:assert/strict'
import { DEFAULT, differs, draftFor, ready } from './model'

const saved = {
  provider: 'openai' as const,
  model: 'gpt-5.5',
  context_window: 272_000,
}

test('switching back to the saved provider restores its model', () => {
  assert.equal(draftFor(DEFAULT, saved), null)
  assert.deepEqual(draftFor('openai', saved), {
    provider: 'openai',
    model: 'gpt-5.5',
    window: 272_000,
  })
  assert.deepEqual(draftFor('anthropic', saved), {
    provider: 'anthropic',
    model: '',
    window: null,
  })
})

test('only a different provider or model is a change worth saving', () => {
  assert.equal(differs(draftFor('openai', saved), saved), false)
  assert.equal(differs(null, saved), true)
  assert.equal(differs(null, null), false)
  assert.equal(ready(draftFor('anthropic', saved)), false)
  assert.equal(ready(null), true)
})
