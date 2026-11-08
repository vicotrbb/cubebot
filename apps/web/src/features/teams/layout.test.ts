// @ts-expect-error bun provides this at test time
import { describe, expect, test } from 'bun:test'
import { graphOf, laidOut } from './layout'
import type { Team } from './model'

const team = (
  id: string,
  lines: [string, string | null][],
  parent?: [string, string],
): Team => ({
  id,
  name: id,
  description: '',
  instructions: '',
  parent: parent?.[0] ?? null,
  reports_to: parent?.[1] ?? null,
  room: `room-${id}`,
  revision: 1,
  members: lines.map(([agent, reports_to]) => ({
    agent,
    name: agent,
    title: 'T',
    role: '',
    reports_to,
    retired: false,
  })),
})

// The user's installation: Scribe and Keeper sit in both teams.
const org = [
  team('exec', [
    ['scribe', null],
    ['keeper', 'scribe'],
  ]),
  team('marketing', [
    ['scribe', null],
    ['analyst', 'scribe'],
    ['courier', 'scribe'],
    ['keeper', 'courier'],
  ]),
]

describe('org chart graph', () => {
  test('an agent is one node, linked to every team and manager it has', () => {
    const graph = graphOf(org)
    const agents = graph.nodes.filter((node) => node.kind === 'agent')
    expect(agents.map((node) => node.ref).sort()).toEqual([
      'analyst',
      'courier',
      'keeper',
      'scribe',
    ])
    const intoKeeper = graph.edges.filter((edge) => edge.to === 'agent:keeper')
    expect(intoKeeper.map((edge) => edge.from).sort()).toEqual([
      'agent:courier',
      'agent:scribe',
    ])
    const intoScribe = graph.edges.filter((edge) => edge.to === 'agent:scribe')
    expect(intoScribe.map((edge) => edge.from).sort()).toEqual([
      'team:exec',
      'team:marketing',
    ])
  })

  test('layers follow the deepest line and nodes in a layer never overlap', () => {
    const placed = laidOut(graphOf(org), 220).nodes
    const at = (id: string) =>
      placed.find((node) => node.id === id)?.layer ?? -1
    expect(at('agent:scribe')).toBe(1)
    expect(at('agent:courier')).toBe(2)
    expect(at('agent:keeper')).toBe(3)
    for (const node of placed)
      for (const other of placed)
        if (node !== other && node.layer === other.layer)
          expect(Math.abs(node.x - other.x)).toBeGreaterThanOrEqual(219.9)
  })

  test('a line that skips a layer bends around the cards in it', () => {
    const { nodes, routes } = laidOut(graphOf(org), 220)
    const route = routes.get('agent:scribe>agent:keeper') ?? []
    expect(route.map((bend) => bend.layer)).toEqual([2])
    for (const node of nodes.filter((item) => item.layer === 2))
      expect(Math.abs(node.x - (route[0]?.x ?? 0))).toBeGreaterThanOrEqual(
        127.9,
      )
  })

  test('a child team hangs under its boss, and loops across teams still lay out', () => {
    const nested = [
      team('top', [['cto', null]]),
      team('eng', [['lead', null]], ['top', 'cto']),
    ]
    const placed = laidOut(graphOf(nested), 200).nodes
    const at = (id: string) =>
      placed.find((node) => node.id === id)?.layer ?? -1
    expect(at('team:eng')).toBe(at('agent:cto') + 1)
    const loop = [
      team('a', [
        ['x', null],
        ['y', 'x'],
      ]),
      team('b', [
        ['y', null],
        ['x', 'y'],
      ]),
    ]
    expect(laidOut(graphOf(loop), 200).nodes).toHaveLength(4)
  })
})
