import { useState } from 'react'
import type { RoomMessage } from '../../api/client'
import { can, type AgentSummary, type Session } from '../../api/types'
import { useAgents } from '../../app/queries'
import { Cube, moodOf } from '../../components/Cube'
import { Attachments } from '../chat/Attachments'
import { Retry, Stop } from '../chat/Conversation'
import { Markdown } from '../chat/Markdown'
import { WorkCard, WorkLog } from '../chat/WorkCard'
import { phrase, tally, type Runs } from './runs'

type Delivery = RoomMessage['deliveries'][number]

const DELIVERY: Record<string, string> = {
  queued: 'queued',
  running: 'working…',
  waiting: 'waiting for a retry',
  delegated: 'waiting on its parts',
  complete: 'replied',
  failed: 'failed',
  cancelled: 'cancelled',
}

const PART: Record<string, string> = {
  queued: 'queued',
  running: 'working',
  delegated: 'working',
  waiting: 'waiting',
  complete: 'done',
  failed: 'failed',
  cancelled: 'stopped',
}

type Shared = { run: Runs; session: Session; agents: AgentSummary[] }

export function AgentReply({
  item,
  run,
  session,
  agents,
}: Shared & { item: Delivery }) {
  const agent = agents.find((candidate) => candidate.id === item.agent_id)
  const parts = run.parts(item.turn)
  return (
    <div className="msg msg--agent agent-message" data-status={item.status}>
      <DeliveryAvatar agent={agent} name={item.agent} />
      <div className="msg__body">
        <div className="msg__head">
          <span className="msg__author">{item.agent}</span>
          {parts.length > 0 && <span className="chip">Lead</span>}
          <span className="faint small">
            · {DELIVERY[item.status] ?? item.status}
          </span>
        </div>
        <Answer item={item} />
        <Notes notes={run.notes(item.turn)} />
        <DeliveryActions item={item} agent={agent} session={session} />
        {parts.length > 0 && (
          <RunParts
            turn={item.turn}
            run={run}
            session={session}
            agents={agents}
          />
        )}
      </div>
    </div>
  )
}

function Answer({ item }: { item: Delivery }) {
  return (
    <>
      {item.terminal.length > 0 && (
        <WorkLog tools={item.terminal} running={item.status === 'running'}>
          {item.terminal.map((tool) => (
            <WorkCard key={tool.id} tool={tool} />
          ))}
        </WorkLog>
      )}
      <Markdown text={item.response ?? item.partial ?? ''} />
      <Attachments turn={item.turn} items={item.attachments} />
      {item.status === 'failed' && item.error && (
        <p className="small run__error">{item.error.replaceAll('_', ' ')}</p>
      )}
    </>
  )
}

function RunParts({ turn, run, session, agents }: Shared & { turn: string }) {
  const [open, setOpen] = useState(false)
  const parts = run.parts(turn)
  const counted = tally(run, turn)
  const list = `run-${turn}`
  return (
    <>
      <button
        type="button"
        className="thread-line run__line"
        aria-expanded={open}
        aria-controls={list}
        onClick={() => setOpen(!open)}
      >
        <span className="run__faces" aria-hidden="true">
          {parts.slice(0, 4).map(({ delivery }) => (
            <Face key={delivery.turn} id={delivery.agent_id} agents={agents} />
          ))}
        </span>
        <span>{phrase(counted)}</span>
        {counted.working > 0 && (
          <span className="dot dot--working" aria-hidden="true" />
        )}
        <span className="faint">{open ? 'Hide' : 'Show'}</span>
      </button>
      {open && (
        <ul className="run__parts" id={list}>
          {parts.map(({ handoff, delivery }) => (
            <Part
              key={delivery.turn}
              handoff={handoff}
              item={delivery}
              run={run}
              session={session}
              agents={agents}
            />
          ))}
        </ul>
      )}
    </>
  )
}

function Part({
  handoff,
  item,
  run,
  session,
  agents,
}: Shared & { handoff: RoomMessage; item: Delivery }) {
  const agent = agents.find((candidate) => candidate.id === item.agent_id)
  const nested = run.parts(item.turn).length > 0
  return (
    <li className="msg msg--agent run__part" data-status={item.status}>
      <DeliveryAvatar agent={agent} name={item.agent} size={28} />
      <div className="msg__body">
        <div className="msg__head">
          <span className="msg__author">{item.agent}</span>
          <span className="run__state">{PART[item.status] ?? item.status}</span>
          <span className="faint small">from {handoff.author}</span>
        </div>
        <Asked text={handoff.message} />
        <Answer item={item} />
        <Notes notes={run.notes(item.turn)} />
        <DeliveryActions item={item} agent={agent} session={session} />
        {nested && (
          <RunParts
            turn={item.turn}
            run={run}
            session={session}
            agents={agents}
          />
        )}
      </div>
    </li>
  )
}

function Asked({ text }: { text: string }) {
  const [whole, setWhole] = useState(false)
  const long = text.length > 220 || text.split('\n').length > 3
  const clamped = long && !whole
  return (
    <div className="run__ask">
      <p className={clamped ? 'run__asked run__asked--clamped' : 'run__asked'}>
        {clamped ? text.replace(/\s+/g, ' ') : text}
      </p>
      {long && (
        <button
          type="button"
          className="link-btn small"
          aria-expanded={whole}
          onClick={() => setWhole(!whole)}
        >
          {whole ? 'Show less' : 'Show the whole hand-off'}
        </button>
      )}
    </div>
  )
}

function Notes({ notes }: { notes: RoomMessage[] }) {
  if (notes.length === 0) return null
  return (
    <ul className="steering">
      {notes.map((note) => (
        <li key={note.id} className="small">
          <strong>{note.author} added</strong>: “{note.message}”
        </li>
      ))}
    </ul>
  )
}

function Face({ id, agents }: { id: string; agents: AgentSummary[] }) {
  const agent = agents.find((candidate) => candidate.id === id)
  return (
    <Cube
      agentId={agent?.id}
      avatar={agent?.avatar ?? null}
      mood="idle"
      size={18}
    />
  )
}

function DeliveryActions({
  item,
  agent,
  session,
}: {
  item: Delivery
  agent: AgentSummary | undefined
  session: Session
}) {
  const open = ['queued', 'running', 'delegated', 'waiting'].includes(
    item.status,
  )
  if (!agent || !open || !can(agent, 'conversation_send', session)) return null
  const turn = { id: item.turn, status: item.status }
  return (
    <div className="msg__actions">
      {item.status === 'waiting' && item.error !== 'budget_exhausted' && (
        <Retry conversation={item.agent_id} turn={turn} />
      )}
      <Stop
        conversation={item.agent_id}
        turn={turn}
        label={item.status === 'queued' ? 'Cancel' : 'Stop'}
      />
    </div>
  )
}

export function DeliveryAvatar({
  agent,
  name,
  size = 32,
}: {
  agent: ReturnType<typeof useAgents>['data'] extends (infer A)[] | undefined
    ? A | undefined
    : never
  name: string
  size?: number
}) {
  if (agent)
    return (
      <Cube
        agentId={agent.id}
        avatar={agent.avatar}
        mood={moodOf(agent)}
        size={size}
        className="msg__avatar"
      />
    )
  return (
    <span className="avatar-initial msg__avatar" aria-hidden="true">
      {name.slice(0, 1).toUpperCase()}
    </span>
  )
}
