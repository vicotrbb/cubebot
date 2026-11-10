import test from 'node:test'
import assert from 'node:assert/strict'
import { plural, uptime } from './facts'

test('uptime names the two largest units', () => {
  assert.equal(uptime(30), 'under a minute')
  assert.equal(uptime(61), '1 minute')
  assert.equal(uptime(3 * 86_400 + 4 * 3_600 + 5 * 60), '3 days, 4 hours')
  assert.equal(uptime(86_400 + 120), '1 day, 2 minutes')
})

test('plural counts nouns', () => {
  assert.equal(plural(1, 'room'), '1 room')
  assert.equal(plural(0, 'team'), '0 teams')
  assert.equal(plural(2, 'person', 'people'), '2 people')
})
