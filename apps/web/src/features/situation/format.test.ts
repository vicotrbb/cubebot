// @ts-expect-error bun provides this at test time
import { describe, expect, test } from 'bun:test'
import { count, fillDays, headline, money, pace } from './format'

test('money never shows real spend as nothing', () => {
  expect(money(0)).toBe('$0.00')
  expect(money(0.004)).toBe('<$0.01')
  expect(money(12.345)).toBe('$12.35')
  expect(money(1234.5)).toBe('$1,235')
})

test('count', () => {
  expect(count(930)).toBe('930')
  expect(count(12_400)).toBe('12.4K')
  expect(count(3_210_000)).toBe('3.2M')
})

test('pace', () => {
  expect(pace(5, null)).toBe('none')
  expect(pace(5, 10)).toBe('fine')
  expect(pace(8, 10)).toBe('near')
  expect(pace(10, 10)).toBe('over')
})

describe('headline', () => {
  const idle = { activity: 'idle', pending_login: false }
  const base = { work: [], failedToday: 0, overBudget: 0 }
  test('is calm only when nothing needs a person', () => {
    const line = headline({ ...base, agents: [idle, idle] })
    expect(line).toEqual({
      calm: true,
      text: 'Nothing needs you.',
      state: '2 agents, none working.',
    })
  })
  test('names what needs a person, most urgent first', () => {
    const line = headline({
      agents: [{ activity: 'working', pending_login: true }, idle],
      work: [{ status: 'waiting' }, { status: 'running' }],
      failedToday: 2,
      overBudget: 1,
    })
    expect(line.calm).toBe(false)
    expect(line.text).toBe(
      '1 turn is waiting, 1 login is waiting, 2 turns failed today, 1 agent is over budget.',
    )
    expect(line.state).toBe('1 of 2 agents working.')
  })
})

test('fillDays leaves no gaps, so an idle day reads as idle', () => {
  const filled = fillDays(
    [{ day: '2026-11-03', tokens: 5 }],
    (day) => ({ day, tokens: 0 }),
    new Date('2026-11-04T12:00:00Z'),
    3,
  )
  expect(filled).toEqual([
    { day: '2026-11-02', tokens: 0 },
    { day: '2026-11-03', tokens: 5 },
    { day: '2026-11-04', tokens: 0 },
  ])
})
