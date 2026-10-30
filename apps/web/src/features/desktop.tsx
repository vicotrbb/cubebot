import { useCallback, useEffect, useRef, useState } from 'react'
import type { Agent } from '../api/client'

/** The guest screen (ADR 0085); frames and input use its pixels. */
const WIDTH = 1280
const HEIGHT = 800
const SAMPLES = 20

type Mode = 'agent' | 'watch' | 'human'
type Action = { action: string } & Record<string, unknown>
type Frame = {
  mode: Mode
  tiles?: { x: number; y: number; png: string }[]
  pointer?: { at: [number, number]; account?: string }
}
type Answer = { ack: number; mode?: Mode; error?: string }
type Point = [number, number]
type Press = { button: number; start: Point; path: Point[] }

export function Desktop({
  agent,
  accountId,
  onClose,
}: {
  agent: Agent
  accountId: string
  onClose: () => void
}) {
  const [watching, setWatching] = useState(false)
  const [status, setStatus] = useState('')
  const [mode, setMode] = useState<Mode | null>(null)
  const [holding, setHolding] = useState(false)
  const [pointer, setPointer] = useState<Frame['pointer']>()
  const [error, setError] = useState('')
  const [latency, setLatency] = useState<number[]>([])
  const [updates, setUpdates] = useState(0)
  const canvas = useRef<HTMLCanvasElement>(null)
  const socket = useRef<WebSocket | null>(null)
  // The `t` of an unanswered take; its answer says whether this viewer now has control.
  const taking = useRef<number | null>(null)

  useEffect(() => {
    if (!watching) return
    const scheme = location.protocol === 'https:' ? 'wss' : 'ws'
    const ws = new WebSocket(
      `${scheme}://${location.host}/api/agents/${agent.id}/desktop`,
    )
    socket.current = ws
    // Frames paint in arrival order, so a tile never shows older pixels than one before it.
    let painting = Promise.resolve()
    let painted = 0
    const rate = setInterval(() => {
      setUpdates(painted)
      painted = 0
    }, 1000)
    ws.onopen = () => setStatus('')
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
        painting = painting.then(async () => {
          if (context) await paint(context, tiles)
        })
      }
    }
    return () => {
      clearInterval(rate)
      ws.close()
      socket.current = null
    }
  }, [watching, agent.id])

  const send = useCallback((message: Record<string, unknown>) => {
    const ws = socket.current
    const t = performance.now()
    if (ws?.readyState === WebSocket.OPEN)
      ws.send(JSON.stringify({ ...message, t }))
    return t
  }, [])
  const input = useCallback(
    (action: Action) => send({ kind: 'input', action }),
    [send],
  )
  useCapture(canvas, holding, input)

  return (
    <section className="desktop" aria-label="Agent desktop">
      <header className="conversation-header">
        <div>
          <h1>{agent.name}: desktop</h1>
          <p className="muted" role="status">
            {status || modeLabel(watching, mode, holding)}
          </p>
        </div>
        <div className="actions">
          <button
            className={watching ? 'secondary' : undefined}
            onClick={() => {
              setStatus(watching ? '' : 'Connecting…')
              setWatching(!watching)
            }}
          >
            {watching ? 'Stop watching' : 'Watch'}
          </button>
          {watching && !holding && (
            <button
              disabled={mode === null || mode === 'human'}
              onClick={() => {
                taking.current = send({ kind: 'take' })
              }}
            >
              Take control
            </button>
          )}
          {holding && (
            <button
              onClick={() => {
                send({ kind: 'hand_back' })
                setHolding(false)
              }}
            >
              Hand back
            </button>
          )}
          <button className="secondary" onClick={onClose}>
            Back to conversation
          </button>
        </div>
      </header>
      {error && <p role="alert">{error}</p>}
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
        {pointer && watching && (
          <Pointer pointer={pointer} accountId={accountId} />
        )}
      </div>
      {watching && <Latency samples={latency} updates={updates} />}
    </section>
  )
}

/** Input round trip: page to orchestrator to the sandbox's arbiter and back. */
function Latency({ samples, updates }: { samples: number[]; updates: number }) {
  const last = samples.at(-1)
  const sorted = [...samples].sort((a, b) => a - b)
  const middle = sorted[Math.floor(sorted.length / 2)]
  return (
    <p className="muted desktop-latency">
      Input round trip:{' '}
      {last === undefined || middle === undefined
        ? 'not measured yet'
        : `${Math.round(last)} ms (median ${Math.round(middle)} ms of ${samples.length})`}{' '}
      · Screen updates: {updates}/s
    </p>
  )
}

function modeLabel(watching: boolean, mode: Mode | null, holding: boolean) {
  if (!watching) return 'Not watching'
  if (mode === null) return 'Connecting…'
  if (holding && mode === 'human') return 'You have control'
  if (mode === 'human') return 'A person has control'
  return 'Watching the agent'
}

function Pointer({
  pointer,
  accountId,
}: {
  pointer: NonNullable<Frame['pointer']>
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
    let moving: [number, number] | null = null
    const at = (event: MouseEvent): [number, number] => {
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
