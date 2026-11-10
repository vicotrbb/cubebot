import { Maximize, Minus, Plus } from 'lucide-react'
import {
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type ReactNode,
} from 'react'
import { MAX, PAD, fitted, zoomed, type Point, type View } from './view'

const STEP = 1.25
const DRAG = 4

export function Canvas({
  width,
  height,
  label,
  children,
}: {
  width: number
  height: number
  label: string
  children: ReactNode
}) {
  const frame = useRef<HTMLDivElement>(null)
  const [size, setSize] = useState({ w: 0, h: 0 })
  const [own, setOwn] = useState<View | null>(null)
  const pointers = useRef(new Map<number, Point>())
  const gesture = useRef<{ moved: boolean; last: Point; spread: number }>({
    moved: false,
    last: { x: 0, y: 0 },
    spread: 0,
  })
  const fit = fitted(size, width, height)
  const widthFit = size.w ? Math.min(1, (size.w - PAD * 2) / width) : 1
  const view = own ?? fit
  const min = Math.min(fit.k, 1) / 2
  const live = useRef({ view, min })
  useLayoutEffect(() => {
    live.current = { view, min }
  })

  useLayoutEffect(() => {
    const element = frame.current
    if (!element) return
    const measure = () =>
      setSize({ w: element.clientWidth, h: element.clientHeight })
    measure()
    const observer = new ResizeObserver(measure)
    observer.observe(element)
    return () => observer.disconnect()
  }, [])

  useEffect(() => {
    const element = frame.current
    if (!element) return
    const wheel = (event: WheelEvent) => {
      if (!event.ctrlKey && !event.metaKey) return
      event.preventDefault()
      const box = element.getBoundingClientRect()
      const at = { x: event.clientX - box.left, y: event.clientY - box.top }
      const { view: now, min: floor } = live.current
      setOwn(zoomed(now, Math.exp(-event.deltaY / 240), at, floor))
    }
    element.addEventListener('wheel', wheel, { passive: false })
    return () => element.removeEventListener('wheel', wheel)
  }, [])

  const centre = () => ({ x: size.w / 2, y: size.h / 2 })
  const zoomBy = (factor: number) => setOwn(zoomed(view, factor, centre(), min))
  const spreadOf = () => {
    const [a, b] = [...pointers.current.values()]
    return a && b ? Math.hypot(a.x - b.x, a.y - b.y) : 0
  }
  const local = (event: React.PointerEvent) => {
    const box = event.currentTarget.getBoundingClientRect()
    return { x: event.clientX - box.left, y: event.clientY - box.top }
  }

  function down(event: React.PointerEvent<HTMLDivElement>) {
    if (event.button !== 0) return
    const at = local(event)
    pointers.current.set(event.pointerId, at)
    gesture.current = { moved: false, last: at, spread: spreadOf() }
  }

  function move(event: React.PointerEvent<HTMLDivElement>) {
    const previous = pointers.current.get(event.pointerId)
    if (!previous) return
    const at = local(event)
    pointers.current.set(event.pointerId, at)
    const now = gesture.current
    if (pointers.current.size > 1) {
      const spread = spreadOf()
      if (now.spread > 0) setOwn(zoomed(view, spread / now.spread, at, min))
      now.spread = spread
      now.moved = true
      return
    }
    const dx = at.x - now.last.x
    const dy = at.y - now.last.y
    if (!now.moved && Math.hypot(dx, dy) < DRAG) return
    if (!now.moved) event.currentTarget.setPointerCapture(event.pointerId)
    now.moved = true
    now.last = at
    setOwn({ ...view, x: view.x + dx, y: view.y + dy })
  }

  function up(event: React.PointerEvent<HTMLDivElement>) {
    pointers.current.delete(event.pointerId)
    gesture.current.spread = spreadOf()
  }

  function key(event: React.KeyboardEvent<HTMLDivElement>) {
    if (event.target !== event.currentTarget) return
    const pan: Record<string, Point> = {
      ArrowLeft: { x: 48, y: 0 },
      ArrowRight: { x: -48, y: 0 },
      ArrowUp: { x: 0, y: 48 },
      ArrowDown: { x: 0, y: -48 },
    }
    const by = pan[event.key]
    if (by) setOwn({ ...view, x: view.x + by.x, y: view.y + by.y })
    else if (event.key === '+' || event.key === '=') zoomBy(STEP)
    else if (event.key === '-') zoomBy(1 / STEP)
    else if (event.key === '0') setOwn(null)
    else return
    event.preventDefault()
  }

  return (
    <div className="canvas">
      <div
        ref={frame}
        className="canvas__frame"
        style={{
          height: `clamp(280px, ${Math.ceil(height * widthFit + PAD * 2)}px, 64vh)`,
        }}
        tabIndex={0}
        role="group"
        aria-label={`${label}. Drag to move, pinch or Ctrl and scroll to zoom, arrow keys move, plus and minus zoom, 0 fits.`}
        onPointerDown={down}
        onPointerMove={move}
        onPointerUp={up}
        onPointerCancel={up}
        onDragStart={(event) => event.preventDefault()}
        onClickCapture={(event) => {
          if (!gesture.current.moved) return
          event.preventDefault()
          event.stopPropagation()
          gesture.current.moved = false
        }}
        onKeyDown={key}
      >
        <div
          className="canvas__world"
          style={{
            width,
            height,
            transform: `translate(${view.x}px, ${view.y}px) scale(${view.k})`,
            visibility: size.w ? 'visible' : 'hidden',
          }}
        >
          {children}
        </div>
      </div>
      <div className="canvas__tools" role="toolbar" aria-label="Zoom">
        <button
          type="button"
          className="btn btn--ghost btn--icon btn--sm"
          aria-label="Zoom out"
          disabled={view.k <= min + 0.001}
          onClick={() => zoomBy(1 / STEP)}
        >
          <Minus />
        </button>
        <button
          type="button"
          className="canvas__scale"
          aria-label="Zoom to actual size"
          onClick={() => setOwn(zoomed(view, 1 / view.k, centre(), min))}
        >
          {Math.round(view.k * 100)}%
        </button>
        <button
          type="button"
          className="btn btn--ghost btn--icon btn--sm"
          aria-label="Zoom in"
          disabled={view.k >= MAX - 0.001}
          onClick={() => zoomBy(STEP)}
        >
          <Plus />
        </button>
        <button
          type="button"
          className="btn btn--ghost btn--icon btn--sm"
          aria-label="Fit the whole chart"
          aria-pressed={own === null}
          onClick={() => setOwn(null)}
        >
          <Maximize />
        </button>
      </div>
    </div>
  )
}
