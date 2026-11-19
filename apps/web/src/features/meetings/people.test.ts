import test from 'node:test'
import assert from 'node:assert/strict'
import type { Meeting } from './api'
import { peopleOf } from './people'

const ANA = '00000000-0000-0000-0000-00000000000a'
const FORGE = '00000000-0000-0000-0000-00000000000f'

const invitation = (kind: string, name: string, id: string) => ({
  id,
  kind,
  name,
  listening: false,
  account: kind === 'account' ? id : null,
  agent: kind === 'agent' ? id : null,
})

const meeting = (overrides: Partial<Meeting>) =>
  ({
    state: 'ended',
    ended_at: '2026-11-18T18:01:00Z',
    invitations: [],
    attendance: [],
    ...overrides,
  }) as Meeting

test('the host, an invited agent and a guest who joined all appear once', () => {
  const people = peopleOf(
    meeting({
      invitations: [
        invitation('agent', 'Forge', FORGE),
        invitation('email', 'bob@example.com', 'i2'),
      ],
      attendance: [
        {
          kind: 'account',
          account: ANA,
          name: 'Ana',
          joined_at: '2026-11-18T17:50:00Z',
          left_at: '2026-11-18T17:55:00Z',
        },
        {
          kind: 'agent',
          agent: FORGE,
          name: 'Forge',
          joined_at: '2026-11-18T17:51:00Z',
          left_at: '2026-11-18T18:01:00Z',
        },
        {
          kind: 'account',
          account: ANA,
          name: 'Ana',
          joined_at: '2026-11-18T17:57:00Z',
          left_at: '2026-11-18T18:01:00Z',
        },
        {
          kind: 'guest',
          name: 'Victor (guest)',
          joined_at: '2026-11-18T17:52:00Z',
          left_at: '2026-11-18T18:01:00Z',
        },
      ],
    }),
  )
  assert.deepEqual(
    people.map((one) => [
      one.name,
      Math.round(one.minutes),
      Boolean(one.invitation),
    ]),
    [
      ['Ana', 9, false],
      ['Forge', 10, true],
      ['Victor (guest)', 9, false],
      ['bob@example.com', 0, true],
    ],
  )
})

test('people still in a live meeting come first and count time until now', () => {
  const people = peopleOf(
    meeting({
      state: 'live',
      ended_at: null,
      invitations: [invitation('agent', 'Forge', FORGE)],
      attendance: [
        {
          kind: 'agent',
          agent: FORGE,
          name: 'Forge',
          joined_at: '2026-11-18T17:50:00Z',
          left_at: '2026-11-18T17:52:00Z',
        },
        {
          kind: 'account',
          account: ANA,
          name: 'Ana',
          joined_at: '2026-11-18T17:55:00Z',
          left_at: null,
        },
      ],
    }),
    Date.parse('2026-11-18T18:00:00Z'),
  )
  assert.deepEqual(
    people.map((one) => [one.name, one.here, Math.round(one.minutes)]),
    [
      ['Ana', true, 5],
      ['Forge', false, 2],
    ],
  )
})
