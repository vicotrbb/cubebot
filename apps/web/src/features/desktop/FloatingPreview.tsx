import { useLocation, useNavigate } from '@tanstack/react-router'
import { Maximize2, X } from 'lucide-react'
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
 * A small live window for each agent on its desktop that the viewer may watch, wherever
 * they are in the console. Dismissing one hides it until that agent's next session; the
 * agent's own Desktop route shows no preview of itself.
 */
export function FloatingPreviews({ session }: { session: Session }) {
  const agents = useAgents()
  const location = useLocation()
  const phone = useIsPhone()
  const [dismissed, setDismissed] = useState<Set<string>>(new Set())
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
  if (shown.length === 0) return null
  if (phone)
    return (
      <div className="pip-pills" role="status">
        {shown.map((agent) => (
          <Pill
            key={agent.id}
            agent={agent}
            onDismiss={() => setDismissed(new Set(dismissed).add(agent.id))}
          />
        ))}
      </div>
    )
  return (
    <>
      {shown.map((agent, index) => (
        <Preview
          key={agent.id}
          agent={agent}
          index={index}
          onDismiss={() => setDismissed(new Set(dismissed).add(agent.id))}
        />
      ))}
    </>
  )
}

function Pill({
  agent,
  onDismiss,
}: {
  agent: AgentSummary
  onDismiss: () => void
}) {
  const navigate = useNavigate()
  return (
    <div className="pip-pill">
      <Cube avatar={agent.avatar} mood="desktop" size={20} />
      <button
        type="button"
        className="link-btn"
        onClick={() =>
          void navigate({
            to: '/agents/$agentId/desktop',
            params: { agentId: agent.id },
            search: {},
          })
        }
      >
        {agent.name} is using its desktop · watch
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
  )
}

function Preview({
  agent,
  index,
  onDismiss,
}: {
  agent: AgentSummary
  index: number
  onDismiss: () => void
}) {
  const navigate = useNavigate()
  const canvas = useRef<HTMLCanvasElement>(null)
  const desk = useDesktopSocket(agent.id, true, canvas)
  const [offset, setOffset] = useState<{ x: number; y: number } | null>(null)
  const drag = useRef<{ x: number; y: number; ox: number; oy: number } | null>(
    null,
  )
  const base = {
    right: GAP,
    top: GAP + index * (PIP_W * (HEIGHT / WIDTH) + 40 + GAP),
  }
  const style = offset
    ? { left: offset.x, top: offset.y }
    : { right: base.right, top: base.top }
  const expand = () =>
    void navigate({
      to: '/agents/$agentId/desktop',
      params: { agentId: agent.id },
      search: {},
    })
  return (
    <div
      className="pip"
      style={style}
      role="region"
      aria-label={`${agent.name}'s desktop, live`}
    >
      <div
        className="pip__head"
        onPointerDown={(event) => {
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
