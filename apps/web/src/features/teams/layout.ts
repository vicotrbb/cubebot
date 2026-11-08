import type { Team } from './model'

/** A team label or an agent; an agent is one node however many teams it is in. */
export type ChartNode = {
  id: string
  kind: 'team' | 'agent'
  ref: string
}

/** A line drawn from `from` (above) to `to` (below), in the teams that draw it. */
export type ChartEdge = { from: string; to: string; teams: string[] }

export type Placed = ChartNode & { layer: number; x: number }

const teamNode = (id: string) => `team:${id}`
const agentNode = (id: string) => `agent:${id}`

/**
 * Teams as one graph: each team's label points at its manager, each manager at who reports to
 * them, and a member of a team above points at the label of a team under it.
 */
export function graphOf(teams: Team[]) {
  const known = new Set(teams.map((team) => team.id))
  const nodes = new Map<string, ChartNode>()
  const edges = new Map<string, ChartEdge>()
  const link = (from: string, to: string, team: string) => {
    const key = `${from}>${to}`
    const edge = edges.get(key) ?? { from, to, teams: [] }
    if (!edge.teams.includes(team)) edge.teams.push(team)
    edges.set(key, edge)
  }
  for (const team of teams) {
    nodes.set(teamNode(team.id), {
      id: teamNode(team.id),
      kind: 'team',
      ref: team.id,
    })
    for (const member of team.members) {
      const id = agentNode(member.agent)
      if (!nodes.has(id))
        nodes.set(id, { id, kind: 'agent', ref: member.agent })
      link(
        member.reports_to ? agentNode(member.reports_to) : teamNode(team.id),
        id,
        team.id,
      )
    }
    if (team.parent && team.reports_to && known.has(team.parent))
      link(agentNode(team.reports_to), teamNode(team.id), team.parent)
  }
  return { nodes: [...nodes.values()], edges: [...edges.values()] }
}

/** Each node one layer below its lowest predecessor; a line that closes a loop is ignored. */
function layers(nodes: ChartNode[], edges: ChartEdge[]) {
  const above = new Map<string, string[]>()
  for (const edge of edges)
    above.set(edge.to, [...(above.get(edge.to) ?? []), edge.from])
  const depth = new Map<string, number>()
  const walking = new Set<string>()
  const of = (id: string): number => {
    const known = depth.get(id)
    if (known !== undefined) return known
    walking.add(id)
    const parents = (above.get(id) ?? []).filter((p) => !walking.has(p))
    const value = parents.length ? Math.max(...parents.map(of)) + 1 : 0
    walking.delete(id)
    depth.set(id, value)
    return value
  }
  for (const node of nodes) of(node.id)
  return depth
}

/** Orders each layer by the mean position of its neighbours, sweeping down then up. */
function ordered(rows: string[][], edges: ChartEdge[]) {
  const near = (id: string, up: boolean) =>
    edges
      .filter((e) => (up ? e.to : e.from) === id)
      .map((e) => (up ? e.from : e.to))
  const sweep = (from: number, to: number, step: number, up: boolean) => {
    for (let index = from; index !== to; index += step) {
      const reference = rows[index - step] ?? []
      const score = (id: string, at: number) => {
        const spots = near(id, up)
          .map((other) => reference.indexOf(other))
          .filter((spot) => spot >= 0)
        return spots.length
          ? spots.reduce((sum, spot) => sum + spot, 0) / spots.length
          : at
      }
      const row = rows[index] ?? []
      rows[index] = row
        .map((id, at) => ({ id, key: score(id, at) }))
        .sort((a, b) => a.key - b.key)
        .map((item) => item.id)
    }
  }
  for (let pass = 0; pass < 4; pass += 1) {
    sweep(1, rows.length, 1, true)
    sweep(rows.length - 2, -1, -1, false)
  }
  return rows
}

/** Moves each node toward the mean x of its neighbours, keeping order and `gap` apart. */
function spaced(
  rows: string[][],
  edges: ChartEdge[],
  gap: number,
): Map<string, number> {
  const x = new Map<string, number>()
  for (const row of rows)
    row.forEach((id, index) => x.set(id, (index - (row.length - 1) / 2) * gap))
  const settle = (row: string[], up: boolean) => {
    const desired = row.map((id) => {
      const others = edges
        .filter((e) => (up ? e.to : e.from) === id)
        .map((e) => x.get(up ? e.from : e.to) ?? 0)
      return others.length
        ? others.reduce((sum, value) => sum + value, 0) / others.length
        : (x.get(id) ?? 0)
    })
    const pushed = [...desired]
    for (let index = 1; index < pushed.length; index += 1)
      pushed[index] = Math.max(
        pushed[index] ?? 0,
        (pushed[index - 1] ?? 0) +
          apart(row[index - 1] ?? '', row[index] ?? '', gap),
      )

    // Pushing only rightward drifts a crowded row; centre it back on what its nodes wanted.
    const drift = (sum(pushed) - sum(desired)) / Math.max(row.length, 1)
    row.forEach((id, index) => x.set(id, (pushed[index] ?? 0) - drift))
  }
  for (let pass = 0; pass < 3; pass += 1) {
    rows.slice(1).forEach((row) => settle(row, true))
    rows
      .slice(0, -1)
      .reverse()
      .forEach((row) => settle(row, false))
  }
  return x
}

/** A waypoint only needs room for its line, a card needs a whole slot. */
const footprint = (id: string, gap: number) =>
  id.startsWith('via:') ? 36 : gap
const apart = (a: string, b: string, gap: number) =>
  (footprint(a, gap) + footprint(b, gap)) / 2

const sum = (values: number[]) =>
  values.reduce((total, value) => total + value, 0)

/** A line's bends: one point per layer it passes through between its ends. */
export type Route = { layer: number; x: number }[]

export const routeKey = (edge: ChartEdge) => `${edge.from}>${edge.to}`

/** Splits each line that skips layers into steps through waypoints, so it is placed like a node. */
function stepped(edges: ChartEdge[], depth: Map<string, number>) {
  const steps: ChartEdge[] = []
  const waypoints = new Map<string, string[]>()
  for (const edge of edges) {
    const from = depth.get(edge.from) ?? 0
    const to = depth.get(edge.to) ?? 0
    const via = Array.from(
      { length: Math.max(0, to - from - 1) },
      (_, index) => `via:${routeKey(edge)}:${from + index + 1}`,
    )
    waypoints.set(routeKey(edge), via)
    const chain = [edge.from, ...via, edge.to]
    for (let index = 1; index < chain.length; index += 1)
      steps.push({
        from: chain[index - 1] ?? '',
        to: chain[index] ?? '',
        teams: edge.teams,
      })
    via.forEach((id, index) => depth.set(id, from + index + 1))
  }
  return { steps, waypoints }
}

/** Layers and horizontal centres for every node and line bend, the leftmost at 0. */
export function laidOut(
  graph: { nodes: ChartNode[]; edges: ChartEdge[] },
  gap: number,
): { nodes: Placed[]; routes: Map<string, Route> } {
  const depth = layers(graph.nodes, graph.edges)
  const { steps, waypoints } = stepped(graph.edges, depth)
  const rows: string[][] = []
  for (const id of depth.keys()) {
    const layer = depth.get(id) ?? 0
    rows[layer] = [...(rows[layer] ?? []), id]
  }
  const filled = ordered(
    Array.from(rows, (row) => row ?? []),
    steps,
  )
  const x = spaced(filled, steps, gap)
  const left = Math.min(...x.values())
  const routes = new Map<string, Route>()
  for (const [key, via] of waypoints)
    routes.set(
      key,
      via.map((id) => ({
        layer: depth.get(id) ?? 0,
        x: (x.get(id) ?? 0) - left,
      })),
    )
  return {
    nodes: graph.nodes.map((node) => ({
      ...node,
      layer: depth.get(node.id) ?? 0,
      x: (x.get(node.id) ?? 0) - left,
    })),
    routes,
  }
}
