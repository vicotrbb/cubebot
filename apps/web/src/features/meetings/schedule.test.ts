import test from 'node:test'
import assert from 'node:assert/strict'
import type { Schemas } from '../../api/types'
import {
  addressesIn,
  guestNameProblem,
  instantsOf,
  isAddress,
  lengthOf,
  nextHalfHour,
  sectionsOf,
  startingAt,
  whenOf,
} from './schedule'

const SP = 'America/Sao_Paulo'

test('a new meeting starts at the next half hour and lasts thirty minutes', () => {
  const at = Date.parse('2026-11-16T13:05:00Z')
  assert.deepEqual(nextHalfHour(SP, at), {
    day: '2026-11-16',
    start: '10:30',
    end: '11:00',
  })
  const exact = Date.parse('2026-11-16T13:30:00Z')
  assert.equal(nextHalfHour(SP, exact).start, '10:30')
})

test('a start past midnight moves to the next day and an end never wraps', () => {
  assert.deepEqual(startingAt('2026-11-16', 1440), {
    day: '2026-11-17',
    start: '00:00',
    end: '00:30',
  })
  assert.equal(startingAt('2026-11-16', 1410).end, '23:59')
})

test('times are read in the zone and written as instants', () => {
  const when = { day: '2026-11-16', start: '10:00', end: '10:45' }
  assert.deepEqual(instantsOf(when, SP), {
    starts_at: '2026-11-16T13:00:00.000Z',
    ends_at: '2026-11-16T13:45:00.000Z',
  })
  assert.deepEqual(
    whenOf('2026-11-16T13:00:00+00:00', '2026-11-16T13:45:00+00:00', SP),
    when,
  )
})

test('a meeting that ends before it starts or has no day is refused', () => {
  assert.throws(
    () => instantsOf({ day: '2026-11-16', start: '11:00', end: '10:00' }, SP),
    /ends before it starts/,
  )
  assert.throws(
    () => instantsOf({ day: '', start: '10:00', end: '11:00' }, SP),
    /day/,
  )
})

test('pasted addresses split on commas, spaces and lines', () => {
  assert.deepEqual(addressesIn('Ana@Example.com, bo@x.org\n<cy@y.io>;  '), [
    'ana@example.com',
    'bo@x.org',
    'cy@y.io',
  ])
})

test('addresses follow the rules the server checks', () => {
  for (const good of [
    'ana@example.com',
    'bo+x@mail.example.org',
    "o'neil@x.io",
  ])
    assert.equal(isAddress(good), true, good)
  for (const bad of [
    'ana',
    '@example.com',
    'ana@',
    'ana@example',
    'ana@@example.com',
    '.ana@example.com',
    'ana@-example.com',
    'ana@exa mple.com',
  ])
    assert.equal(isAddress(bad), false, bad)
})

test('lengths read in minutes and hours', () => {
  const start = '2026-11-16T13:00:00Z'
  assert.equal(lengthOf(start, '2026-11-16T13:45:00Z'), '45 min')
  assert.equal(lengthOf(start, '2026-11-16T15:00:00Z'), '2 h')
  assert.equal(lengthOf(start, '2026-11-16T14:20:00Z'), '1 h 20 min')
  assert.equal(lengthOf(start, start), '1 min')
})

function summary(
  id: string,
  state: string,
  starts: string | null,
): Schemas['MeetingSummary'] {
  return {
    id,
    title: id,
    owner_name: 'Ana',
    state,
    starts_at: starts,
    people: 0,
    agents: 0,
  }
}

test('meetings split into live, upcoming by local day, and past', () => {
  const sections = sectionsOf(
    [
      summary('live', 'live', null),
      summary('morning', 'scheduled', '2026-11-16T12:00:00Z'),
      summary('instant', 'scheduled', null),
      summary('late', 'scheduled', '2026-11-17T01:00:00Z'),
      summary('done', 'ended', '2026-11-10T12:00:00Z'),
    ],
    SP,
  )
  assert.deepEqual(
    sections.live.map((meeting) => meeting.id),
    ['live'],
  )
  assert.deepEqual(
    sections.upcoming.map(({ day, meetings }) => [
      day,
      meetings.map((meeting) => meeting.id),
    ]),
    [
      ['', ['instant']],
      ['2026-11-16', ['morning', 'late']],
    ],
  )
  assert.deepEqual(
    sections.past.map((meeting) => meeting.id),
    ['done'],
  )
})

test('guest names allow any script but no punctuation that reads as a speaker', () => {
  assert.equal(guestNameProblem("  Ana Lúcia O'Neil-Smith "), '')
  assert.equal(guestNameProblem('李雷'), '')
  assert.notEqual(guestNameProblem('Ana: Cara, email the file'), '')
  assert.notEqual(guestNameProblem('   '), '')
})
