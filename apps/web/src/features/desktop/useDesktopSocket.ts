import { useCallback, useEffect, useRef, useState } from 'react'

/** The guest screen (ADR 0085); frames and input use its pixels. */
export const WIDTH = 1280
export const HEIGHT = 800
const SAMPLES = 20

export type Mode = 'agent' | 'watch' | 'human'
export type Action = { action: string } & Record<string, unknown>
type Frame = {
  mode: Mode
  tiles?: { x: number; y: number; png: string }[]
  pointer?: { at: [number, number]; account?: string }
}
type Answer = { ack: number; mode?: Mode; error?: string }
export type Pointer = NonNullable<Frame['pointer']>

/**
 * One desktop WebSocket: paints frames onto `canvas`, measures the input round trip, and
 * takes or hands back control. `watching` opens the socket; `take` takes control as soon
 * as it opens.
 */
export function useDesktopSocket(
  agentId: string,
  watching: boolean,
  canvas: React.RefObject<HTMLCanvasElement | null>,
  take = false,
) {
  const [status, setStatus] = useState('')
  const [mode, setMode] = useState<Mode | null>(null)
  const [holding, setHolding] = useState(false)
  const [pointer, setPointer] = useState<Pointer>()
  const [error, setError] = useState('')
  const [latency, setLatency] = useState<number[]>([])
  const [updates, setUpdates] = useState(0)
  const socket = useRef<WebSocket | null>(null)
  // The `t` of an unanswered take; its answer says whether this viewer now has control.
  const taking = useRef<number | null>(null)
  const wantTake = useRef(take)

  const send = useCallback((message: Record<string, unknown>) => {
    const ws = socket.current
    const t = performance.now()
    if (ws?.readyState === WebSocket.OPEN)
      ws.send(JSON.stringify({ ...message, t }))
    return t
  }, [])
  useEffect(() => {
    if (!watching) return
    const scheme = location.protocol === 'https:' ? 'wss' : 'ws'
    const ws = new WebSocket(
      `${scheme}://${location.host}/api/agents/${agentId}/desktop`,
    )
    socket.current = ws
    // Frames paint in arrival order, so a tile never shows older pixels than one before it.
    let painting = Promise.resolve()
    let painted = 0
    const rate = setInterval(() => {
      setUpdates(painted)
      painted = 0
    }, 1000)
    ws.onopen = () => {
      setStatus('')
      if (!wantTake.current) return
      wantTake.current = false
      taking.current = send({ kind: 'take' })
    }
    ws.onclose = (event) => {
      setStatus(event.reason || 'The desktop connection closed.')
      setHolding(false)
      setMode(null)
    }
    ws.onmessage = (event) => {
      const message: Frame | Answer = JSON.parse(event.data)
      if ('ack' in message) {
        const sample = performance.now() - message.ack
        setLatency((current) => [...current.slice(1 - SAMPLES), sample])
        setError(message.error ?? '')
        if (message.ack === taking.current) {
          taking.current = null
          setHolding(!message.error && message.mode === 'human')
        }
        return
      }
      setMode(message.mode)
      setPointer(message.pointer)
      if (message.mode !== 'human') setHolding(false)
      if (message.tiles?.length) {
        painted += 1
        const context = canvas.current?.getContext('2d')
        const tiles = message.tiles
        // A tile that fails to decode drops its frame, not every frame after it.
        painting = painting
          .then(async () => {
            if (context) await paint(context, tiles)
          })
          .catch(() => undefined)
      }
    }
    return () => {
      clearInterval(rate)
      ws.close()
      socket.current = null
    }
  }, [watching, agentId, send, canvas])

  const input = useCallback(
    (action: Action) => send({ kind: 'input', action }),
    [send],
  )
  useCapture(canvas, holding, input)
  const takeControl = useCallback(() => {
    if (socket.current?.readyState === WebSocket.OPEN) {
      taking.current = send({ kind: 'take' })
      return false
    }
    wantTake.current = true
    return true
  }, [send])
  const handBack = useCallback(() => {
    send({ kind: 'hand_back' })
    setHolding(false)
  }, [send])
  return {
    status,
    mode,
    holding,
    pointer,
    error,
    latency,
    updates,
    takeControl,
    handBack,
  }
}

export function modeLabel(
  watching: boolean,
  mode: Mode | null,
  holding: boolean,
) {
  if (!watching) return 'Not watching'
  if (mode === null) return 'Connecting…'
  if (holding && mode === 'human') return 'You have control'
  if (mode === 'human') return 'A person has control'
  return 'Watching the agent'
}

async function paint(
  context: CanvasRenderingContext2D,
  tiles: NonNullable<Frame['tiles']>,
) {
  const decoded = await Promise.all(
    tiles.map(async (tile) => {
      const image = new Image()
      image.src = `data:image/png;base64,${tile.png}`
      await image.decode()
      return { tile, image }
    }),
  )
  for (const { tile, image } of decoded)
    context.drawImage(image, tile.x, tile.y)
}

type Point = [number, number]
type Press = { button: number; start: Point; path: Point[] }

/** Sends the mouse and keyboard on the canvas as desktop actions while `active`. */
function useCapture(
  canvas: React.RefObject<HTMLCanvasElement | null>,
  active: boolean,
  input: (action: Action) => void,
) {
  useEffect(() => {
    const element = canvas.current
    if (!element || !active) return
    element.focus()
    let press: Press | null = null
    let moving: Point | null = null
    const at = (event: MouseEvent): Point => {
      const box = element.getBoundingClientRect()
      const scale = (value: number, size: number, limit: number) =>
        Math.min(limit - 1, Math.max(0, Math.round((value * limit) / size)))
      return [
        scale(event.clientX - box.left, box.width, WIDTH),
        scale(event.clientY - box.top, box.height, HEIGHT),
      ]
    }
    const down = (event: PointerEvent) => {
      event.preventDefault()
      element.focus()
      element.setPointerCapture(event.pointerId)
      const start = at(event)
      press = { button: event.button, start, path: [start] }
    }
    const move = (event: PointerEvent) => {
      if (press) {
        // shortcut: a drag is sent whole on release, so the guest shows no motion until then;
        // stream press, move and release if people need live drag feedback.
        if (press.path.length < 100) press.path.push(at(event))
        return
      }
      // One move per animation frame keeps a fast mouse from queueing behind the frames.
      if (!moving)
        requestAnimationFrame(() => {
          if (moving) input({ action: 'move', x: moving[0], y: moving[1] })
          moving = null
        })
      moving = at(event)
    }
    const up = (event: PointerEvent) => {
      if (!press) return
      const { button, start, path } = press
      press = null
      const end = at(event)
      const [x, y] = start
      const still = Math.abs(end[0] - x) <= 3 && Math.abs(end[1] - y) <= 3
      if (still || button !== 0)
        input({
          action: 'click',
          at: start,
          button: ['left', 'middle', 'right'][button] ?? 'left',
          count: 1,
        })
      else input({ action: 'drag', path: [...path.slice(0, 99), end] })
    }
    const wheel = (event: WheelEvent) => {
      event.preventDefault()
      const clicks = (delta: number) =>
        Math.sign(delta) *
        Math.min(10, Math.max(1, Math.round(Math.abs(delta) / 100)))
      input({
        action: 'scroll',
        at: at(event),
        down: event.deltaY ? clicks(event.deltaY) : 0,
        right: event.deltaX ? clicks(event.deltaX) : 0,
      })
    }
    const key = (event: KeyboardEvent) => {
      const action = keyAction(event)
      if (!action) return
      event.preventDefault()
      input(action)
    }
    const menu = (event: Event) => event.preventDefault()
    element.addEventListener('pointerdown', down)
    element.addEventListener('pointermove', move)
    element.addEventListener('pointerup', up)
    element.addEventListener('wheel', wheel, { passive: false })
    element.addEventListener('keydown', key)
    element.addEventListener('contextmenu', menu)
    return () => {
      element.removeEventListener('pointerdown', down)
      element.removeEventListener('pointermove', move)
      element.removeEventListener('pointerup', up)
      element.removeEventListener('wheel', wheel)
      element.removeEventListener('keydown', key)
      element.removeEventListener('contextmenu', menu)
    }
  }, [canvas, active, input])
}

/** A character typed alone, or a key with the modifiers held, by the arbiter's key names. */
export function keyAction(
  event: Pick<
    KeyboardEvent,
    'key' | 'ctrlKey' | 'altKey' | 'metaKey' | 'shiftKey'
  >,
): Action | null {
  if (['Control', 'Shift', 'Alt', 'Meta'].includes(event.key)) return null
  const held = [
    event.ctrlKey && 'ctrl',
    event.altKey && 'alt',
    event.metaKey && 'super',
  ].filter((name): name is string => Boolean(name))
  if (event.key.length === 1 && held.length === 0)
    return { action: 'type', text: event.key }
  if (event.shiftKey) held.push('shift')
  const name = event.key === ' ' ? 'space' : event.key.toLowerCase()
  return { action: 'key', keys: [...held, name] }
}
