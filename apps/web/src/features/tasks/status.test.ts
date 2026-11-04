// @ts-expect-error bun provides this at test time
import { expect, test } from 'bun:test'
import {
  doneCount,
  pileOf,
  piles,
  planWindow,
  sentenceOf,
  titleOf,
  type Task,
} from './status'

const task = (extra: Partial<Task> = {}): Task => ({
  id: 't',
  room_id: 'r',
  title: 'Find venues',
  request: 'Find venues\nfor 18 people',
  status: 'working',
  blocked: false,
  reading: false,
  owner: 'victor',
  created_at: '',
  updated_at: '',
  agents: [{ id: 'a', name: 'Ayla' }],
  items: [],
  ...extra,
})
const item = (status: string) => ({
  id: status,
  agent_id: 'a',
  position: 1,
  title: 'Search',
  status,
  note: null,
})

test('a stuck agent or a question comes before anything else', () => {
  expect(pileOf(task())).toBe('working')
  expect(pileOf(task({ blocked: true }))).toBe('needs_you')
  expect(pileOf(task({ status: 'needs_you' }))).toBe('needs_you')
  expect(pileOf(task({ status: 'done' }))).toBe('finished')
  expect(pileOf(task({ status: 'stopped' }))).toBe('finished')
  expect(piles([task(), task({ status: 'done' })]).finished.length).toBe(1)
})

test('a task without a name yet goes by its first line', () => {
  expect(titleOf(task())).toBe('Find venues')
  expect(titleOf(task({ title: null }))).toBe('Find venues')
})

test('the sentence says what is happening', () => {
  expect(sentenceOf(task())).toBe('The orchestrator is planning')
  expect(sentenceOf(task({ reading: true }))).toBe(
    'The orchestrator is reading',
  )
  expect(sentenceOf(task({ items: [item('doing')] }))).toBe('Ayla: Search')
  expect(sentenceOf(task({ items: [item('done'), item('todo')] }))).toBe(
    '1 of 2 done',
  )
  expect(doneCount(task({ items: [item('skipped'), item('todo')] }))).toBe(1)
})

test('the card shows the part of a long plan where the work is', () => {
  const plan = (statuses: string[]) =>
    statuses.map((status, index) => ({
      id: String(index),
      task_id: 't',
      agent_id: 'a',
      position: index,
      title: `Item ${index}`,
      status,
      note: null,
    }))
  const short = planWindow(plan(['done', 'doing', 'todo']))
  expect([short.shown.length, short.before, short.after]).toEqual([3, 0, 0])
  const middle = planWindow(
    plan([
      'done',
      'done',
      'done',
      'doing',
      'todo',
      'todo',
      'todo',
      'todo',
      'todo',
    ]),
  )
  expect(middle.shown[0]?.title).toBe('Item 2')
  expect([middle.before, middle.after]).toEqual([2, 2])
  const finished = planWindow(plan(Array(7).fill('done')))
  expect([finished.before, finished.after]).toEqual([2, 0])
  const fresh = planWindow(plan(Array(7).fill('todo')))
  expect([fresh.before, fresh.after]).toEqual([0, 2])
})
