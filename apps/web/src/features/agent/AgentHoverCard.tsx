import { useEffect, useLayoutEffect, useRef, useSyncExternalStore } from 'react'
import { createPortal } from 'react-dom'
import {
  currentHover,
  setHover,
  subscribeHover,
} from '../../components/agentHover'
import { Cube, moodOf } from '../../components/Cube'
import { useAgent } from '../../app/queries'
import { ACTIVITY } from '../shell/Sidebar'
import { doingOf } from './doing'
import './hover.css'

/** Away from the pointer, so the card never covers what it describes. */
const OFFSET = 14
const OPEN_DELAY = 280

/** Keeps the card by the point and on screen, on the point's other side near an edge. */
function place(card: HTMLElement, x: number, y: number) {
  const { width, height } = card.getBoundingClientRect()
  const left =
    x + OFFSET + width > window.innerWidth ? x - OFFSET - width : x + OFFSET
  const top =
    y + OFFSET + height > window.innerHeight ? y - OFFSET - height : y + OFFSET
  card.style.transform = `translate(${Math.max(4, left)}px, ${Math.max(4, top)}px)`
}

/** The one small card for whatever agent the pointer is on, wherever that agent appears. */
export function AgentHoverCard() {
  const hovered = useSyncExternalStore(subscribeHover, currentHover, () => null)
  const card = useRef<HTMLDivElement>(null)
  const shown = useRef(false)
  useEffect(() => {
    if (!hovered) {
      shown.current = false
      return
    }
    // Between agents a card is already up, so the next one opens at once.
    const delay = shown.current ? 0 : OPEN_DELAY
    const open = window.setTimeout(() => {
      shown.current = true
      card.current?.setAttribute('data-open', 'true')
    }, delay)
    let { x, y } = hovered
    const leave = () => setHover(null)
    const move = (event: PointerEvent) => {
      if (!hovered.anchor.isConnected) return leave()
      if (hovered.focused) return
      if (!hovered.anchor.contains(event.target as Node)) return leave()
      x = event.clientX
      y = event.clientY
      if (card.current) place(card.current, x, y)
    }
    const pointerLeftPage = (event: PointerEvent) => {
      if (!event.relatedTarget && !hovered.focused) leave()
    }
    const key = (event: KeyboardEvent) => {
      if (event.key === 'Escape') leave()
    }
    window.addEventListener('pointermove', move, { passive: true })
    document.addEventListener('pointerout', pointerLeftPage)
    window.addEventListener('pointerdown', leave)
    window.addEventListener('blur', leave)
    window.addEventListener('scroll', leave, { capture: true, passive: true })
    window.addEventListener('keydown', key)
    return () => {
      window.clearTimeout(open)
      window.removeEventListener('pointermove', move)
      document.removeEventListener('pointerout', pointerLeftPage)
      window.removeEventListener('pointerdown', leave)
      window.removeEventListener('blur', leave)
      window.removeEventListener('scroll', leave, { capture: true })
      window.removeEventListener('keydown', key)
    }
  }, [hovered])
  useLayoutEffect(() => {
    if (hovered && card.current) place(card.current, hovered.x, hovered.y)
  })
  if (!hovered) return null
  return createPortal(<Card ref={card} id={hovered.id} />, document.body)
}

function Card({ id, ref }: { id: string; ref: React.Ref<HTMLDivElement> }) {
  const { agent } = useAgent(id)
  if (!agent) return null
  const role = agent.instructions?.split('\n').find((line) => line.trim())
  return (
    <div ref={ref} className="agent-card" role="tooltip">
      <div className="agent-card__head">
        <Cube avatar={agent.avatar} mood={moodOf(agent)} size={36} />
        <div className="agent-card__who">
          <strong>{agent.name}</strong>
          <span className="agent-card__status">
            <span className={`dot dot--${agent.activity}`} />
            {ACTIVITY[agent.activity] ?? agent.activity}
          </span>
        </div>
      </div>
      <p className="agent-card__doing">{doingOf(agent)}</p>
      {role && <p className="agent-card__role">{role}</p>}
    </div>
  )
}
