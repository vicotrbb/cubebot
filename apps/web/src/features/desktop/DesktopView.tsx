import { useQueryClient } from '@tanstack/react-query'
import { Link } from '@tanstack/react-router'
import { ChevronLeft, Hand, Pause, Play, Power } from 'lucide-react'
import { useRef, useState } from 'react'
import { admin } from '../../api/admin'
import { failure } from '../../api/client'
import type { AgentSummary } from '../../api/types'
import { keys } from '../../app/queries'
import { Cube, moodOf } from '../../components/Cube'
import { Alert } from '../../components/primitives'
import { useIsPhone } from '../shell/viewport'
import { FillPicker } from '../vault/FillPicker'
import { LoginBanner } from './LoginBanner'
import {
  HEIGHT,
  WIDTH,
  modeLabel,
  useDesktopSocket,
  type Mode,
  type Pointer,
} from './useDesktopSocket'
import './desktop.css'

/** The full desktop (docs/adr/sandbox.md#live-browser-viewing-and-explicit-human-takeover): watch, take control, hand back, fill from the vault. */
export function DesktopView({
  agent,
  accountId,
  take,
  canControl,
  canManage,
}: {
  agent: AgentSummary
  accountId: string
  /** Watches and takes control as soon as the connection opens. */
  take?: boolean | undefined
  canControl: boolean
  canManage: boolean
}) {
  const [watching, setWatching] = useState(true)
  const phone = useIsPhone()
  const canvas = useRef<HTMLCanvasElement>(null)
  const running = agent.desired_state === 'running'
  const desk = useDesktopSocket(
    agent.id,
    watching && running,
    canvas,
    Boolean(take),
  )
  const takeControl = () => {
    if (desk.takeControl()) setWatching(true)
  }
  return (
    <section className="desktop" aria-label="Agent desktop">
      <header className="agent-head">
        <Link
          to="/agents/$agentId"
          params={{ agentId: agent.id }}
          className="btn btn--ghost btn--icon"
          aria-label="Back to conversation"
        >
          <ChevronLeft />
        </Link>
        <Cube avatar={agent.avatar} mood={moodOf(agent)} size={32} />
        <div className="agent-head__title">
          <h1>{agent.name}: desktop</h1>
          <p className="faint small" role="status">
            {running
              ? desk.status || modeLabel(watching, desk.mode, desk.holding)
              : 'Computer off'}
          </p>
        </div>
        <Controls
          watching={watching}
          mode={desk.mode}
          holding={desk.holding}
          canTake={canControl && !phone}
          onWatch={() => setWatching(!watching)}
          onTake={takeControl}
          onHandBack={desk.handBack}
        />
      </header>
      <LoginBanner agent={agent} holding={desk.holding} onTake={takeControl} />
      <FillPicker agent={agent} holding={desk.holding} />
      <Alert>{desk.error}</Alert>
      <div className="desktop-stage">
        <div className="desktop-screen">
          <canvas
            ref={canvas}
            width={WIDTH}
            height={HEIGHT}
            tabIndex={desk.holding ? 0 : -1}
            role="img"
            aria-label={`Agent desktop${desk.holding ? '. You have control: mouse and keyboard go to the agent.' : ''}`}
            className={desk.holding ? 'controlling' : undefined}
          />
          {desk.pointer && watching && (
            <PointerMark pointer={desk.pointer} accountId={accountId} />
          )}
          {watching && !desk.live && (
            <ScreenCover
              agent={agent}
              running={running}
              reason={desk.status}
              canManage={canManage}
            />
          )}
        </div>
      </div>
      {watching && <Latency samples={desk.latency} updates={desk.updates} />}
      {phone && canControl && (
        <p className="faint small" style={{ padding: '0 16px 12px' }}>
          Taking control needs a mouse and keyboard; watch here and take over
          from a desktop.
        </p>
      )}
    </section>
  )
}

function Controls({
  watching,
  mode,
  holding,
  canTake,
  onWatch,
  onTake,
  onHandBack,
}: {
  watching: boolean
  mode: Mode | null
  holding: boolean
  canTake: boolean
  onWatch: () => void
  onTake: () => void
  onHandBack: () => void
}) {
  return (
    <div className="agent-head__actions">
      <button type="button" className="btn btn--sm" onClick={onWatch}>
        {watching ? <Pause /> : <Play />}
        {watching ? 'Stop watching' : 'Watch'}
      </button>
      {canTake && watching && !holding && (
        <button
          type="button"
          className="btn btn--primary btn--sm"
          disabled={mode === null || mode === 'human'}
          onClick={onTake}
        >
          <Hand />
          Take control
        </button>
      )}
      {holding && (
        <button
          type="button"
          className="btn btn--primary btn--sm"
          onClick={onHandBack}
        >
          Hand back
        </button>
      )}
    </div>
  )
}

/** What the screen shows before its first frame: the computer off, or starting. */
function ScreenCover({
  agent,
  running,
  reason,
  canManage,
}: {
  agent: AgentSummary
  running: boolean
  reason: string
  canManage: boolean
}) {
  const client = useQueryClient()
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  const retired = agent.desired_state === 'retired'
  async function start() {
    setBusy(true)
    setError('')
    try {
      await admin.lifecycle(agent.id, 'start', agent.revision)
      await client.invalidateQueries({ queryKey: keys.agents })
    } catch (error) {
      setError(failure(error))
    } finally {
      setBusy(false)
    }
  }
  if (running)
    return (
      <div className="desktop-cover" aria-live="polite">
        <Cube avatar={agent.avatar} mood="desktop" size={64} />
        <p className="desktop-cover__title">
          {reason
            ? `Waiting for ${agent.name}'s computer…`
            : `Connecting to ${agent.name}'s desktop…`}
        </p>
        <p className="desktop-cover__note">
          {reason
            ? `${reason.replace(/^./, (first) => first.toUpperCase())}. Trying again.`
            : 'The first picture arrives in a moment.'}
        </p>
      </div>
    )
  return (
    <div className="desktop-cover">
      <Cube avatar={agent.avatar} mood={moodOf(agent)} size={64} />
      <p className="desktop-cover__title">
        {retired
          ? `${agent.name} is retired.`
          : `${agent.name}'s computer is off.`}
      </p>
      {!retired && (
        <p className="desktop-cover__note">
          {canManage
            ? 'Start it to see its desktop. Apps and files are where it left them.'
            : 'A message starts it, or ask a manager of this agent to.'}
        </p>
      )}
      {!retired && canManage && (
        <button
          type="button"
          className="btn btn--primary"
          disabled={busy}
          onClick={() => void start()}
        >
          <Power />
          {busy ? 'Starting…' : `Start ${agent.name}`}
        </button>
      )}
      <Alert>{error}</Alert>
    </div>
  )
}

/** Input round trip: page to orchestrator to the sandbox's arbiter and back. */
function Latency({ samples, updates }: { samples: number[]; updates: number }) {
  const last = samples.at(-1)
  const sorted = [...samples].sort((a, b) => a - b)
  const middle = sorted[Math.floor(sorted.length / 2)]
  return (
    <p className="faint small desktop-latency tabular">
      Input round trip:{' '}
      {last === undefined || middle === undefined
        ? 'not measured yet'
        : `${Math.round(last)} ms (median ${Math.round(middle)} ms of ${samples.length})`}{' '}
      · Screen updates: {updates}/s
    </p>
  )
}

function PointerMark({
  pointer,
  accountId,
}: {
  pointer: Pointer
  accountId: string
}) {
  const source = !pointer.account
    ? 'agent'
    : pointer.account === accountId
      ? 'you'
      : 'person'
  const labels = { agent: 'Agent', you: 'You', person: 'Person' }
  return (
    <span
      className={`desktop-pointer ${source}`}
      style={{
        left: `${(pointer.at[0] / WIDTH) * 100}%`,
        top: `${(pointer.at[1] / HEIGHT) * 100}%`,
      }}
      aria-hidden="true"
    >
      {labels[source]}
    </span>
  )
}
