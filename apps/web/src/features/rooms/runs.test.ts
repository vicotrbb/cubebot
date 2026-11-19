// @ts-expect-error bun provides this at test time
import { expect, test } from 'bun:test'
import type { RoomMessage } from '../../api/client'
import { phrase, runs, tally } from './runs'

type Turn = [turn: string, status: string, parent?: string]

const message = (
  id: string,
  turns: Turn[] = [],
  agent: string | null = null,
  steered: string | null = null,
) =>
  ({
    id,
    author_agent_id: agent,
    steered,
    deliveries: turns.map(([turn, status, parent]) => ({
      turn,
      status,
      parent_turn: parent ?? null,
    })),
  }) as unknown as RoomMessage

const room = [
  message('ask', [['lead', 'delegated']]),
  message('to bee', [['bee', 'complete', 'lead']], 'ada'),
  message('to cy', [['cy', 'running', 'lead']], 'ada'),
  message('cy asks dee', [['dee', 'failed', 'cy']], 'cy'),
  message('ada adds', [], 'ada', 'cy'),
  message('next ask', [['other', 'queued']]),
]

test('a run folds its hand-offs and peer notes under the turn that sent them', () => {
  const run = runs(room)
  expect(run.top.map((item) => item.id)).toEqual(['ask', 'next ask'])
  expect(run.parts('lead').map((part) => part.delivery.turn)).toEqual([
    'bee',
    'cy',
  ])
  expect(run.parts('cy').map((part) => part.handoff.id)).toEqual([
    'cy asks dee',
  ])
  expect(run.notes('cy').map((note) => note.id)).toEqual(['ada adds'])
  expect(run.parts('other')).toEqual([])
})

test('a run counts every part below the lead by how it stands', () => {
  const counted = tally(runs(room), 'lead')
  expect(counted).toEqual({
    total: 3,
    done: 1,
    working: 1,
    failed: 1,
    waiting: 0,
  })
  expect(phrase(counted)).toBe('3 parts · 1 done · 1 working · 1 failed')
})

test('a hand-off whose sender is not loaded stays in the timeline', () => {
  const run = runs([message('to bee', [['bee', 'running', 'gone']], 'ada')])
  expect(run.top.map((item) => item.id)).toEqual(['to bee'])
})
