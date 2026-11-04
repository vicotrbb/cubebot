import { useLocation, useNavigate } from '@tanstack/react-router'
import { Layers, Maximize2, X } from 'lucide-react'
import { useRef, useState } from 'react'
import { can, type AgentSummary, type Session } from '../../api/types'
import { useAgents } from '../../app/queries'
import { Cube } from '../../components/Cube'
import { useIsPhone } from '../shell/viewport'
import { HEIGHT, WIDTH, useDesktopSocket } from './useDesktopSocket'
import './desktop.css'

const PIP_W = 320
const GAP = 12

/**
 * A small live window of the agents on their desktops that the viewer may watch, wherever they are
 * in the console. Several stack on top of each other with one live at the front; opening the stack
 * picks which to watch. Dismissing one hides it until that agent's next session; the agent's own
 * Desktop route shows no preview of itself.
 */
export function FloatingPreviews({ session }: { session: Session }) {
  const agents = useAgents()
  const location = useLocation()
  const phone = useIsPhone()
  const [dismissed, setDismissed] = useState<Set<string>>(new Set())
  const [front, setFront] = useState<string | null>(null)
  const active = (agents.data ?? []).filter(
    (agent) => agent.desktop_active && can(agent, 'browser_view', session),
  )
  // A dismissal ends with the session it dismissed: prune ids no longer active.
  const live = new Set(active.map((agent) => agent.id))
  const kept = new Set([...dismissed].filter((id) => live.has(id)))
  if (kept.size !== dismissed.size) setDismissed(kept)
  const shown = active.filter(
    (agent) =>
      !dismissed.has(agent.id) &&
      !location.pathname.startsWith(`/agents/${agent.id}/desktop`),
  )
  const current = shown.find((agent) => agent.id === front) ?? shown[0]
  if (!current) return null
  const dismiss = (agent: AgentSummary) =>
    setDismissed(new Set(dismissed).add(agent.id))
  if (phone)
    return (
      <PhoneStack
        current={current}
        agents={shown}
        onPick={setFront}
        onDismiss={dismiss}
      />
    )
  return (
    <Preview
      key={current.id}
      agent={current}
      agents={shown}
      onPick={setFront}
      onDismiss={() => dismiss(current)}
    />
  )
}

/** The agent's desktop page. */
function useOpenDesktop() {
  const navigate = useNavigate()
  return (agent: AgentSummary) =>
    void navigate({
      to: '/agents/$agentId/desktop',
      params: { agentId: agent.id },
      search: {},
    })
}

/** Everyone in the stack, to pick from. */
function Picker({
  agents,
  current,
  onPick,
}: {
  agents: AgentSummary[]
  current: AgentSummary
  onPick: (id: string) => void
}) {
  return (
    <div className="pip__picker" role="group" aria-label="Choose a desktop">
      {agents.map((agent) => (
        <button
          key={agent.id}
          type="button"
          className="pip__choice"
          aria-current={agent.id === current.id ? 'true' : undefined}
          onClick={() => onPick(agent.id)}
        >
          <Cube avatar={agent.avatar} mood="desktop" size={20} />
          <span>
            {agent.name}
            {agent.pending_login ? ' · needs you' : ''}
          </span>
        </button>
      ))}
    </div>
  )
}

/** A phone has no room for a window: one line, opening to the list when there are several. */
function PhoneStack({
  current,
  agents,
  onPick,
  onDismiss,
}: {
  current: AgentSummary
  agents: AgentSummary[]
  onPick: (id: string) => void
  onDismiss: (agent: AgentSummary) => void
}) {
  const [open, setOpen] = useState(false)
  const openDesktop = useOpenDesktop()
  const others = agents.length - 1
  return (
    <div className="pip-pills" role="status">
      <div className="pip-pill">
        <Cube avatar={current.avatar} mood="desktop" size={20} />
        <button
          type="button"
          className="link-btn"
          onClick={() => openDesktop(current)}
        >
          {current.name} is using its desktop · watch
        </button>
        {others > 0 && (
          <button
            type="button"
            className="pip__count"
            aria-expanded={open}
            aria-label={`${others} more desktops`}
            onClick={() => setOpen(!open)}
          >
            +{others}
          </button>
        )}
        <button
          type="button"
          className="btn btn--ghost btn--icon btn--sm"
          aria-label="Dismiss"
          onClick={() => onDismiss(current)}
        >
          <X />
        </button>
      </div>
      {open && (
        <div className="pip-pill pip-pill--list">
          <Picker
            agents={agents}
            current={current}
            onPick={(id) => {
              setOpen(false)
              onPick(id)
            }}
          />
        </div>
      )}
    </div>
  )
}

function Preview({
  agent,
  agents,
  onPick,
  onDismiss,
}: {
  agent: AgentSummary
  agents: AgentSummary[]
  onPick: (id: string) => void
  onDismiss: () => void
}) {
  const openDesktop = useOpenDesktop()
  // A task's plan and Stop sit along the top right; start beside them, not over them.
  const clear = useLocation({
    select: (location) => (location.pathname.startsWith('/tasks/') ? 300 : 0),
  })
  const [picking, setPicking] = useState(false)
  const behind = agents.length - 1
  const canvas = useRef<HTMLCanvasElement>(null)
  const desk = useDesktopSocket(agent.id, true, canvas)
  const [offset, setOffset] = useState<{ x: number; y: number } | null>(null)
  const drag = useRef<{ x: number; y: number; ox: number; oy: number } | null>(
    null,
  )
  const style = offset
    ? { left: offset.x, top: offset.y }
    : {
        right: GAP + clear,
        top: GAP + (clear ? 50 : 0) + (behind > 0 ? 15 : 0),
      }
  const expand = () => openDesktop(agent)
  return (
    <div
      className={behind > 0 ? 'pip pip--stacked' : 'pip'}
      style={style}
      role="region"
      aria-label={`${agent.name}'s desktop, live`}
    >
      {behind > 0 && <span className="pip__behind pip__behind--1" />}
      {behind > 1 && <span className="pip__behind pip__behind--2" />}
      <div
        className="pip__head"
        onPointerDown={(event) => {
          // Capturing the pointer would retarget the click away from Expand and Dismiss.
          if ((event.target as Element).closest('button')) return
          const box = event.currentTarget.parentElement?.getBoundingClientRect()
          if (!box) return
          drag.current = {
            x: event.clientX,
            y: event.clientY,
            ox: box.left,
            oy: box.top,
          }
          event.currentTarget.setPointerCapture(event.pointerId)
        }}
        onPointerMove={(event) => {
          const start = drag.current
          if (!start) return
          setOffset({
            x: Math.max(
              0,
              Math.min(
                window.innerWidth - PIP_W,
                start.ox + event.clientX - start.x,
              ),
            ),
            y: Math.max(
              0,
              Math.min(
                window.innerHeight - 60,
                start.oy + event.clientY - start.y,
              ),
            ),
          })
        }}
        onPointerUp={() => {
          drag.current = null
        }}
      >
        <Cube avatar={agent.avatar} mood="desktop" size={20} />
        <span className="pip__name">{agent.name}</span>
        <span className="dot dot--working" aria-hidden="true" />
        <span className="faint small">
          {desk.mode === 'human' ? 'a person has control' : 'live'}
        </span>
        {behind > 0 && (
          <button
            type="button"
            className="pip__count"
            aria-expanded={picking}
            aria-label={`Choose among ${agents.length} desktops`}
            title="Choose a desktop"
            onClick={() => setPicking(!picking)}
          >
            <Layers aria-hidden="true" />
            {agents.length}
          </button>
        )}
        <button
          type="button"
          className="btn btn--ghost btn--icon btn--sm"
          aria-label="Expand"
          onClick={expand}
        >
          <Maximize2 />
        </button>
        <button
          type="button"
          className="btn btn--ghost btn--icon btn--sm"
          aria-label="Dismiss"
          onClick={onDismiss}
        >
          <X />
        </button>
      </div>
      {picking && <Picker agents={agents} current={agent} onPick={onPick} />}
      <button
        type="button"
        className="pip__screen"
        onClick={expand}
        aria-label="Open the desktop"
      >
        <canvas ref={canvas} width={WIDTH} height={HEIGHT} />
      </button>
    </div>
  )
}
