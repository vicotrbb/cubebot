// @ts-expect-error bun provides this at test time
import { describe, expect, test } from 'bun:test'
import {
  bossChoices,
  makeManager,
  problems,
  removeMember,
  type SaveTeam,
} from './model'

const team = (lines: [string, string | null][]): SaveTeam => ({
  name: 'Platform',
  description: '',
  instructions: '',
  parent: null,
  reports_to: null,
  members: lines.map(([agent, reports_to]) => ({
    agent,
    title: 'Engineer',
    role: '',
    reports_to,
  })),
})

describe('team drafts', () => {
  test('a new manager takes the old one under it', () => {
    const changed = makeManager(
      team([
        ['lia', null],
        ['dev', 'lia'],
      ]),
      'dev',
    )
    expect(changed.members).toMatchObject([
      { agent: 'lia', reports_to: 'dev' },
      { agent: 'dev', reports_to: null },
    ])
    expect(problems(changed, [])).toEqual([])
  })

  test('removing a member hands its reports to its boss and keeps the manager', () => {
    const draft = team([
      ['lia', null],
      ['mid', 'lia'],
      ['dev', 'mid'],
    ])
    expect(removeMember(draft, 'mid').members).toMatchObject([
      { agent: 'lia', reports_to: null },
      { agent: 'dev', reports_to: 'lia' },
    ])
    expect(removeMember(draft, 'lia')).toBe(draft)
  })

  test('nobody may report to someone below them', () => {
    const draft = team([
      ['lia', null],
      ['mid', 'lia'],
      ['dev', 'mid'],
    ])
    expect(bossChoices(draft, 'mid').map((m) => m.agent)).toEqual(['lia'])
  })

  test('a draft names what keeps it from saving', () => {
    const draft = { ...team([['lia', null]]), name: ' ' }
    expect(problems(draft, [])).toEqual(['Name the team.'])
    expect(problems(team([]), [])).toEqual(['Add at least one agent.'])
  })
})
