// @ts-expect-error bun provides this at test time
import { describe, expect, test } from 'bun:test'
import { pulledOf, stepsOf, totalsOf, type Trace } from './trace'

const at = (seconds: number) =>
  new Date(Date.UTC(2026, 10, 18, 12, 0, seconds)).toISOString()

const trace = (events: Trace['events'], status = 'complete'): Trace => ({
  turn: 't',
  status,
  provider: 'anthropic',
  model: 'claude-sonnet-5-5',
  created_at: at(0),
  completed_at: at(30),
  events,
  memories: [],
})

const usage = (
  sequence: number,
  seconds: number,
  stage: 'reply' | 'review' | undefined,
  cost: number | null,
): Trace['events'][number] => ({
  sequence,
  at: at(seconds),
  cost_usd: cost,
  payload: {
    kind: 'usage',
    input_tokens: 1000,
    output_tokens: 100,
    cached_input_tokens: 400,
    ...(stage ? { stage } : {}),
  },
})

const call = (
  sequence: number,
  seconds: number,
  id: string,
  name: string,
  input: unknown,
): Trace['events'][number] => ({
  sequence,
  at: at(seconds),
  payload: { kind: 'tool_call', id, name, input },
})

const result = (
  sequence: number,
  seconds: number,
  id: string,
  output: string,
): Trace['events'][number] => ({
  sequence,
  at: at(seconds),
  payload: {
    kind: 'tool_result',
    id,
    outcome: { status: 'exited', code: 0, output },
  },
})

describe('trace', () => {
  const events: Trace['events'] = [
    usage(1, 2, undefined, 0.01),
    call(2, 2, 'a', 'search_memory', { query: 'gift' }),
    call(3, 2, 'b', 'bash', { command: 'ls' }),
    result(4, 3, 'a', '[{"id":"m1","revision":2,"content":"Chris likes GTA"}]'),
    result(5, 7, 'b', 'README.md'),
    call(6, 8, 'c', 'kb_read', { page: 'p1' }),
    result(7, 9, 'c', '{"id":"p1","title":"Org \\"chart\\"","body":"# Org'),
    usage(8, 10, 'review', null),
    {
      sequence: 9,
      at: at(10),
      payload: { kind: 'review', outcome: 'complete' },
    },
  ]

  test('pairs each call with its result and times it', () => {
    const steps = stepsOf(trace(events))
    expect(steps.map((step) => step.kind)).toEqual([
      'model',
      'tool',
      'tool',
      'tool',
      'model',
      'review',
    ])
    expect(steps[2]).toMatchObject({ took: 5000, tool: { command: 'ls' } })
    expect(steps[0]).toMatchObject({ stage: 'reply' })
    expect(steps[4]).toMatchObject({ stage: 'review' })
  })

  test('a call without a result is running only while the turn runs', () => {
    const open = [call(1, 1, 'x', 'bash', { command: 'sleep 9' })]
    expect(stepsOf(trace(open, 'running'))[0]).toMatchObject({
      tool: { status: 'running' },
    })
    expect(stepsOf(trace(open))[0]).toMatchObject({
      tool: { status: 'uncertain' },
    })
  })

  test('totals tokens and flags unpriced calls', () => {
    const totals = totalsOf(stepsOf(trace(events)))
    expect(totals).toEqual({
      calls: 2,
      tools: 3,
      input: 2000,
      cached: 800,
      output: 200,
      cost: 0.01,
      unpriced: true,
    })
  })

  test('lists what was pulled in, even from a cut page', () => {
    expect(pulledOf(stepsOf(trace(events)))).toEqual([
      {
        kind: 'memory',
        query: 'gift',
        found: [{ id: 'm1', content: 'Chris likes GTA' }],
      },
      { kind: 'kb_page', id: 'p1', title: 'Org "chart"' },
    ])
  })
})
