import test from 'node:test'
import assert from 'node:assert/strict'
import { suggestedWindow, tokens } from './models'

test('the provider report wins over the family size', () => {
  assert.deepEqual(suggestedWindow('claude-new', 1_000_000), {
    tokens: 1_000_000,
    source: 'reported',
  })
  assert.equal(suggestedWindow('gpt-5.5', 900_000)?.source, 'reported')
})

test('OpenAI families get their documented input size', () => {
  assert.equal(suggestedWindow('gpt-4.1-mini', null)?.tokens, 1_047_576)
  assert.equal(suggestedWindow('gpt-5.5', null)?.tokens, 272_000)
  assert.equal(suggestedWindow('o4-mini', undefined)?.tokens, 200_000)
  assert.equal(suggestedWindow('gpt-4o', null)?.tokens, 128_000)
})

test('an unknown model gets no guess', () => {
  assert.equal(suggestedWindow('omni-moderation', null), null)
  assert.equal(suggestedWindow('o10-preview', null), null)
})

test('sizes read in thousands or millions', () => {
  assert.equal(tokens(128_000), '128K')
  assert.equal(tokens(1_047_576), '1.05M')
  assert.equal(tokens(1_000_000), '1M')
})
