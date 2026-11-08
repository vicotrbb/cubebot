import test from 'node:test'
import assert from 'node:assert/strict'
import type { components } from '../../api/schema'
import { backupDate, lastVerified, took } from './backups'

test('a backup id is its UTC start', () => {
  assert.equal(
    backupDate('20261107T030405Z')?.toISOString(),
    '2026-11-07T03:04:05.000Z',
  )
  assert.equal(backupDate('latest'), null)
})

test('the newest verified backup is found past failed and running ones', () => {
  const row = (
    id: string,
    status: string,
  ): components['schemas']['BackupSummary'] => ({
    id,
    status,
    started_at_unix: 0,
    bytes: 0,
    missing: [],
  })
  const found = lastVerified([
    row('c', 'running'),
    row('b', 'failed'),
    row('a', 'incomplete'),
  ])
  assert.equal(found?.id, 'a')
})

test('durations read in the largest units that fit', () => {
  assert.equal(took(0, 42), '42 s')
  assert.equal(took(0, 200), '3 min 20 s')
  assert.equal(took(0, 3 * 3600 + 120), '3 h 2 min')
  assert.equal(took(0, null), '')
})
