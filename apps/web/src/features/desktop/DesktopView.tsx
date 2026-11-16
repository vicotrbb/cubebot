import { useQueryClient } from '@tanstack/react-query'
import { Link } from '@tanstack/react-router'
import {
  Activity,
  ChevronLeft,
  Hand,
  Pause,
  Play,
  Power,
  Undo2,
} from 'lucide-react'
import { useRef, useState, type ReactNode, type RefObject } from 'react'
import { admin } from '../../api/admin'
import { failure } from '../../api/client'
import type { AgentSummary } from '../../api/types'
import { keys } from '../../app/queries'
import { Cube, moodOf } from '../../components/Cube'
import { Alert } from '../../components/primitives'
import { useIsPhone } from '../shell/viewport'
import { FillPicker } from '../vault/FillPicker'
import { VitalsOverlay } from '../vitals/Vitals'
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

export function DesktopView({
  agent,
  accountId,
  take,
  canControl,
  canManage,
}: {
  agent: AgentSummary
  accountId: string
  take?: boolean | undefined
  canControl: boolean
  canManage: boolean
}) {
  const [watching, setWatching] = useState(true)
  const [vitals, setVitals] = useState(remembered)
  const phone = useIsPhone()
  const canvas = useRef<HTMLCanvasElement>(null)
  const running = agent.desired_state === 'running'
  const desk = useDesktopSocket(
    agent.id,
    watching && running,
    canvas,
    Boolean(take),
  )
  const takeable = canControl && !phone
  const takeControl = () => {
    if (desk.takeControl()) setWatching(true)
  }
  return (
    <section className="desktop" aria-label="Agent desktop">
      <Head
        agent={agent}
        status={
          running
            ? desk.status || modeLabel(watching, desk.mode, desk.holding)
            : 'Computer off'
        }
      >
        <Controls
          watching={watching}
          mode={desk.mode}
          holding={desk.holding}
          canTake={takeable && !agent.pending_login}
          onWatch={() => setWatching(!watching)}
          onTake={takeControl}
          onHandBack={desk.handBack}
          vitals={vitals}
          onVitals={() => setVitals(remember(!vitals))}
        />
      </Head>
      <div className="desktop-notices">
        <LoginBanner
          agent={agent}
          holding={desk.holding}
          canTake={takeable && watching}
          onTake={takeControl}
        />
        <FillPicker agent={agent} holding={desk.holding} />
        <Alert>{desk.error}</Alert>
      </div>
      <div className="desktop-stage">
        <Screen canvas={canvas} holding={desk.holding}>
          {vitals && <VitalsOverlay agent={agent} />}
          {watching && (
            <Overlay
              agent={agent}
              desk={desk}
              accountId={accountId}
              running={running}
              canManage={canManage}
            />
          )}
        </Screen>
      </div>
      <footer className="desktop-foot">
        {watching && desk.live && (
          <Latency samples={desk.latency} updates={desk.updates} />
        )}
        {phone && canControl && <PhoneNote />}
      </footer>
    </section>
  )
}

function Screen({
  canvas,
  holding,
  children,
}: {
  canvas: RefObject<HTMLCanvasElement | null>
  holding: boolean
  children: ReactNode
}) {
  return (
    <div className="desktop-screen">
      <canvas
        ref={canvas}
        width={WIDTH}
        height={HEIGHT}
        tabIndex={holding ? 0 : -1}
        role="img"
        aria-label={`Agent desktop${holding ? '. You have control: mouse and keyboard go to the agent.' : ''}`}
        className={holding ? 'controlling' : undefined}
      />
      {children}
    </div>
  )
}

function Head({
  agent,
  status,
  children,
}: {
  agent: AgentSummary
  status: string
  children: ReactNode
}) {
  return (
    <header className="agent-head">
      <Link
        to="/agents/$agentId"
        params={{ agentId: agent.id }}
        className="btn btn--ghost btn--icon"
        aria-label="Back to conversation"
      >
        <ChevronLeft />
      </Link>
      <Cube
        agentId={agent.id}
        avatar={agent.avatar}
        mood={moodOf(agent)}
        size={28}
      />
      <div className="agent-head__title">
        <h1>
          {agent.name} <span className="faint">· Desktop</span>
        </h1>
        <p className="faint small" role="status">
          {status}
        </p>
      </div>
      {children}
    </header>
  )
}

function Overlay({
  agent,
  desk,
  accountId,
  running,
  canManage,
}: {
  agent: AgentSummary
  desk: ReturnType<typeof useDesktopSocket>
  accountId: string
  running: boolean
  canManage: boolean
}) {
  return (
    <>
      {desk.pointer && (
        <PointerMark pointer={desk.pointer} accountId={accountId} />
      )}
      {!desk.live && (
        <ScreenCover
          agent={agent}
          running={running}
          reason={desk.status}
          canManage={canManage}
        />
      )}
    </>
  )
}

function PhoneNote() {
  return (
    <p className="faint small">
      Taking control needs a mouse and keyboard; watch here and take over from a
      computer.
    </p>
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
  vitals,
  onVitals,
}: {
  watching: boolean
  mode: Mode | null
  holding: boolean
  canTake: boolean
  onWatch: () => void
  onTake: () => void
  onHandBack: () => void
  vitals: boolean
  onVitals: () => void
}) {
  return (
    <div className="agent-head__actions">
      <button
        type="button"
        className={
          vitals
            ? 'btn btn--sm desktop-vitals-toggle on'
            : 'btn btn--sm btn--ghost desktop-vitals-toggle'
        }
        aria-pressed={vitals}
        aria-label="Vitals overlay"
        title={
          vitals
            ? 'Hide CPU, memory, storage and network'
            : 'Show CPU, memory, storage and network'
        }
        onClick={onVitals}
      >
        <Activity />
        <span className="agent-head__word">Vitals</span>
      </button>
      <button
        type="button"
        className={holding ? 'btn btn--ghost btn--sm' : 'btn btn--sm'}
        aria-label={watching ? 'Stop watching' : 'Watch'}
        onClick={onWatch}
      >
        {watching ? <Pause /> : <Play />}
        <span className="agent-head__word">
          {watching ? 'Stop watching' : 'Watch'}
        </span>
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
          <Undo2 />
          Hand back
        </button>
      )}
    </div>
  )
}

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

function Latency({ samples, updates }: { samples: number[]; updates: number }) {
  const last = samples.at(-1)
  const sorted = [...samples].sort((a, b) => a - b)
  const middle = sorted[Math.floor(sorted.length / 2)]
  return (
    <p className="desktop-latency tabular" role="status">
      <span className="desktop-latency__live" aria-hidden="true" />
      Live · {updates} {updates === 1 ? 'update' : 'updates'}/s
      {last !== undefined && middle !== undefined && (
        <>
          {' '}
          · input {Math.round(last)} ms
          <span className="faint"> (median {Math.round(middle)} ms)</span>
        </>
      )}
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

const VITALS_KEY = 'cubebot:desktop-vitals'

function remembered() {
  try {
    return localStorage.getItem(VITALS_KEY) === 'on'
  } catch {
    return false
  }
}

function remember(on: boolean) {
  try {
    localStorage.setItem(VITALS_KEY, on ? 'on' : 'off')
  } catch {
    return on
  }
  return on
}
