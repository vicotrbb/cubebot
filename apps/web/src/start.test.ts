import assert from 'node:assert/strict'
import test from 'node:test'
import { frameAncestors } from './start'

test('embedding is refused unless valid sources are configured', () => {
  assert.equal(frameAncestors(undefined), "'none'")
  assert.equal(frameAncestors('  '), "'none'")
  assert.equal(
    frameAncestors('https://ha.lan:8123 https://*.example.com'),
    'https://ha.lan:8123 https://*.example.com',
  )
  for (const unsafe of [
    "https://a.lan; script-src 'unsafe-inline'",
    '*',
    "'self' x",
  ])
    assert.equal(frameAncestors(unsafe), "'none'", unsafe)
})
