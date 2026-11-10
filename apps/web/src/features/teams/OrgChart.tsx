import { Link } from '@tanstack/react-router'
import { UsersRound } from 'lucide-react'
import { useState } from 'react'
import type { AgentSummary } from '../../api/types'
import { agentHover } from '../../components/agentHover'
import { Cube, moodOf } from '../../components/Cube'
import { doingOf } from '../agent/doing'
import {
  graphOf,
  laidOut,
  routeKey,
  type ChartEdge,
  type Placed,
  type Route,
} from './layout'
import { teamsBelow, type Team, type TeamMember } from './model'
import { Canvas } from './Canvas'
import './teams.css'

const WIDTH = 252
const SLOT = WIDTH + 32
const ROW_GAP = 46
const TEAM_HEIGHT = 32

type Seat = { team: Team; member: TeamMember }

/**
 * The organization as one graph: every agent once, with a line to it from each team label or
 * manager it answers to. Without `root`, every team, then the agents in none.
 */
export function OrgChart({
  teams,
  agents,
  root,
}: {
  teams: Team[]
  agents: AgentSummary[]
  root?: string
}) {
  const [focus, setFocus] = useState<string | null>(null)
  const shown = root
    ? teams.filter((team) => teamsBelow(teams, root).has(team.id))
    : teams
  const seats = new Map<string, Seat[]>()
  for (const team of shown)
    for (const member of team.members)
      seats.set(member.agent, [
        ...(seats.get(member.agent) ?? []),
        { team, member },
      ])
  const named = new Map(agents.map((agent) => [agent.id, agent]))
  const loose = root ? [] : agents.filter((agent) => !seats.has(agent.id))
  return (
    <div className="org">
      {shown.length === 0 && !root && (
        <p className="org__empty">
          No teams yet. A team gives agents a manager, shared instructions and a
          room of their own.
        </p>
      )}
      {shown.length > 0 && (
        <Graph
          teams={shown}
          seats={seats}
          agents={named}
          focus={focus}
          onFocus={setFocus}
        />
      )}
      {loose.length > 0 && (
        <section className="org__loose" aria-labelledby="org-loose">
          <h4 id="org-loose">Not in a team</h4>
          <ul>
            {loose.map((agent) => (
              <li key={agent.id}>
                <AgentCard
                  id={agent.id}
                  name={agent.name}
                  seats={[]}
                  agent={agent}
                />
              </li>
            ))}
          </ul>
        </section>
      )}
    </div>
  )
}

function heightOf(
  node: Placed,
  seats: Map<string, Seat[]>,
  agents: Map<string, AgentSummary>,
) {
  if (node.kind === 'team') return TEAM_HEIGHT
  const lines =
    Math.max(1, seats.get(node.ref)?.length ?? 1) +
    (agents.get(node.ref)?.title ? 1 : 0)
  return 58 + 18 * lines
}

function Graph({
  teams,
  seats,
  agents,
  focus,
  onFocus,
}: {
  teams: Team[]
  seats: Map<string, Seat[]>
  agents: Map<string, AgentSummary>
  focus: string | null
  onFocus: (team: string | null) => void
}) {
  const graph = graphOf(teams)
  const { nodes: placed, routes } = laidOut(graph, SLOT)
  const depth = Math.max(...placed.map((node) => node.layer)) + 1
  const rows = Array.from({ length: depth }, (_, layer) =>
    Math.max(
      0,
      ...placed
        .filter((node) => node.layer === layer)
        .map((node) => heightOf(node, seats, agents)),
    ),
  )
  const tops = rows.map((_, layer) =>
    rows.slice(0, layer).reduce((sum, height) => sum + height + ROW_GAP, 0),
  )
  const at = new Map(placed.map((node) => [node.id, node]))
  const box = (id: string) => {
    const node = at.get(id)
    const top = node ? (tops[node.layer] ?? 0) : 0
    const bottom = node ? top + heightOf(node, seats, agents) : 0
    return { x: (node?.x ?? 0) + WIDTH / 2, top, bottom }
  }
  const bends = [...routes.values()].flat().map((bend) => bend.x)
  const width = Math.max(...placed.map((node) => node.x), ...bends) + WIDTH
  const height = (tops.at(-1) ?? 0) + (rows.at(-1) ?? 0)
  const lit = (edge: ChartEdge) => focus !== null && edge.teams.includes(focus)
  const touched = new Set(
    graph.edges.filter(lit).flatMap((edge) => [edge.from, edge.to]),
  )
  return (
    <Canvas width={width} height={height} label="Organization chart">
      <div
        className="org__graph"
        style={{ width, height }}
        onMouseLeave={() => onFocus(null)}
      >
        <svg width={width} height={height} aria-hidden="true">
          {graph.edges.map((edge) => {
            const from = box(edge.from)
            const to = box(edge.to)
            const through = (routes.get(routeKey(edge)) ?? []).map((bend) =>
              band(bend, tops, rows),
            )
            return (
              <path
                key={routeKey(edge)}
                className="org__line"
                data-lit={lit(edge)}
                d={pathOf({ x: from.x, y: from.bottom }, through, {
                  x: to.x,
                  y: to.top,
                })}
              />
            )
          })}
        </svg>
        {placed.map((node) => (
          <div
            key={node.id}
            className="org__slot"
            data-dim={focus !== null && !touched.has(node.id)}
            style={{
              left: node.x,
              top: tops[node.layer],
              width: WIDTH,
              height: heightOf(node, seats, agents),
            }}
          >
            {node.kind === 'team' ? (
              <TeamLabel
                team={teams.find((team) => team.id === node.ref)}
                onFocus={onFocus}
              />
            ) : (
              <AgentCard
                id={node.ref}
                name={seats.get(node.ref)?.[0]?.member.name ?? 'An agent'}
                seats={seats.get(node.ref) ?? []}
                agent={agents.get(node.ref)}
              />
            )}
          </div>
        ))}
      </div>
    </Canvas>
  )
}

type Point = { x: number; y: number }

/** Where a line crosses a layer it skips: straight down through that layer's band. */
function band(bend: Route[number], tops: number[], rows: number[]) {
  const top = tops[bend.layer] ?? 0
  return [
    { x: bend.x + WIDTH / 2, y: top },
    { x: bend.x + WIDTH / 2, y: top + (rows[bend.layer] ?? 0) },
  ] as const
}

/** Curves between layers, straight runs through the bands of skipped layers. */
function pathOf(
  start: Point,
  through: (readonly [Point, Point])[],
  end: Point,
) {
  const curve = (a: Point, b: Point) => {
    const bend = (b.y - a.y) / 2
    return `C ${a.x} ${a.y + bend}, ${b.x} ${b.y - bend}, ${b.x} ${b.y}`
  }
  let at = start
  let d = `M ${start.x} ${start.y}`
  for (const [enter, leave] of through) {
    d += ` ${curve(at, enter)} L ${leave.x} ${leave.y}`
    at = leave
  }
  return `${d} ${curve(at, end)}`
}

/** Pointing at a team label lights its lines and dims the rest. */
function TeamLabel({
  team,
  onFocus,
}: {
  team: Team | undefined
  onFocus: (team: string | null) => void
}) {
  if (!team) return null
  return (
    <Link
      to="/teams/$teamId"
      params={{ teamId: team.id }}
      className="org__team"
      onMouseEnter={() => onFocus(team.id)}
      onFocus={() => onFocus(team.id)}
      onBlur={() => onFocus(null)}
    >
      <UsersRound aria-hidden="true" />
      <span>{team.name}</span>
      <small>{team.members.length}</small>
    </Link>
  )
}

/** One card per agent; with several teams, each title names the team it belongs to. */
function AgentCard({
  id,
  name,
  seats,
  agent,
}: {
  id: string
  name: string
  seats: Seat[]
  agent: AgentSummary | undefined
}) {
  const retired = seats.some((seat) => seat.member.retired)
  const body = (
    <>
      <Cube
        avatar={agent?.avatar ?? null}
        mood={agent ? moodOf(agent) : retired ? 'retired' : 'idle'}
        size={30}
      />
      <span className="org__who">
        <strong>{name}</strong>
        {agent?.title && (
          <span className="org__role" title={agent.title}>
            {agent.title}
          </span>
        )}
        {seats.map((seat) => (
          <span
            className="org__title"
            key={seat.team.id}
            title={`${seat.member.title}, ${seat.team.name}`}
          >
            {seat.member.title}
            {seats.length > 1 && <small> · {seat.team.name}</small>}
          </span>
        ))}
        <span className="org__doing">
          {agent ? (
            <>
              <span
                className={`dot dot--${agent.activity}`}
                aria-hidden="true"
              />
              {doingOf(agent)}
            </>
          ) : retired ? (
            'Retired'
          ) : (
            'Not shared with you'
          )}
        </span>
      </span>
    </>
  )
  return agent ? (
    <Link
      to="/agents/$agentId"
      params={{ agentId: id }}
      className="org__agent"
      {...agentHover(id)}
    >
      {body}
    </Link>
  ) : (
    <span className="org__agent" data-muted="true">
      {body}
    </span>
  )
}
