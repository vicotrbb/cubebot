// @ts-expect-error bun provides this at test time
import { expect, test } from 'bun:test'
import { byUrgency, excerpt, type Item } from './digest'

test('an excerpt reads a titled report as one line', () => {
  expect(
    excerpt(
      '**Daily health overview, Mon Nov 16**\n\n1. **Whoop:** recovery **62%**, HRV 73.9 ms.\n- See [the dashboard](https://x.y)',
    ),
  ).toBe(
    'Daily health overview, Mon Nov 16 · Whoop: recovery 62%, HRV 73.9 ms. · See the dashboard',
  )
})

test('an excerpt drops code and table rules but keeps snake_case', () => {
  expect(
    excerpt(
      '## Done\n```sh\nrm -rf /\n```\n| a | b |\n|---|---|\nfixed read_cursor',
    ),
  ).toBe('Done · | a | b | · fixed read_cursor')
})

test('a long excerpt is cut on a word', () => {
  const cut = excerpt('word '.repeat(80), 40)
  expect(cut.endsWith('word…')).toBe(true)
  expect(cut.length).toBeLessThanOrEqual(41)
})

test('what needs you sorts questions first, then newest', () => {
  const item = (kind: string, at: string) => ({ kind, at }) as Item
  const sorted = byUrgency([
    item('turn_failed', '2026-11-17T10:00:00Z'),
    item('attention', '2026-11-16T10:00:00Z'),
    item('attention', '2026-11-17T09:00:00Z'),
    item('approval_pending', '2026-11-17T11:00:00Z'),
  ])
  expect(
    sorted.map((entry) => `${entry.kind}@${entry.at.slice(8, 10)}`),
  ).toEqual([
    'attention@17',
    'attention@16',
    'approval_pending@17',
    'turn_failed@17',
  ])
})
