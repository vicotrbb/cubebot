import { useEffect, useRef } from 'react'
import { agentHover } from './agentHover'
import './cube.css'

export const PAINTS = [
  'orange',
  'sage',
  'paper',
  'wood',
  'sky',
  'rose',
  'lemon',
  'plum',
] as const
export const EYES = ['round', 'wide', 'sleepy', 'dot'] as const
export const ACCESSORIES = ['none', 'cap', 'glasses', 'bow', 'antenna'] as const
export type Paint = (typeof PAINTS)[number]
export type Eyes = (typeof EYES)[number]
export type Accessory = (typeof ACCESSORIES)[number]
export type Avatar = { paint: Paint; eyes: Eyes; accessory: Accessory }
export type Mood =
  | 'idle'
  | 'working'
  | 'dreaming'
  | 'sleeping'
  | 'stopped'
  | 'desktop'
  | 'waiting'
  | 'retired'

export const DEFAULT_AVATAR: Avatar = {
  paint: 'orange',
  eyes: 'round',
  accessory: 'none',
}

export function moodOf(agent: {
  activity: string
  desktop_active?: boolean
  pending_login?: boolean
}): Mood {
  if (agent.activity === 'retired') return 'retired'
  if (agent.pending_login) return 'waiting'
  if (agent.desktop_active) return 'desktop'
  if (agent.activity === 'working') return 'working'
  if (agent.activity === 'dreaming') return 'dreaming'
  if (agent.activity === 'sleeping') return 'sleeping'
  if (agent.activity === 'stopped') return 'stopped'
  return 'idle'
}

function cubeClasses(
  paint: Paint,
  mood: Mood,
  follow: boolean,
  className: string | undefined,
) {
  return [
    'cube',
    `cube--${paint}`,
    `cube--${mood}`,
    follow && 'cube--follow',
    className,
  ]
    .filter(Boolean)
    .join(' ')
}

export function Cube({
  avatar = DEFAULT_AVATAR,
  mood = 'idle',
  size = 36,
  follow = false,
  className,
  agentId,
}: {
  avatar?: Avatar | null
  mood?: Mood
  size?: number
  follow?: boolean
  className?: string
  agentId?: string | undefined
}) {
  const ref = useRef<HTMLDivElement>(null)
  useFollow(ref, follow)
  const look = avatar ?? DEFAULT_AVATAR
  const paint = mood === 'retired' ? 'wood' : look.paint
  const eyes = mood === 'retired' ? null : look.eyes
  const classes = cubeClasses(paint, mood, follow, className)
  return (
    <div
      ref={ref}
      className={classes}
      style={{ '--size': `${size}px` } as React.CSSProperties}
      aria-hidden="true"
      {...(agentId ? agentHover(agentId) : {})}
    >
      <div className="cube__body">
        <div className="cube__face cube__face--front">
          {eyes && <EyePair eyes={eyes} mood={mood} />}
          {eyes && mood !== 'sleeping' && mood !== 'stopped' && (
            <>
              <span className="cube__blush cube__blush--l" />
              <span className="cube__blush cube__blush--r" />
            </>
          )}
          {mood !== 'retired' && <Wear accessory={look.accessory} />}
        </div>
        <div className="cube__face cube__face--right" />
        <div className="cube__face cube__face--left" />
        <div className="cube__face cube__face--back" />
        <div className="cube__face cube__face--top" />
        <div className="cube__face cube__face--bottom" />
      </div>
      {mood === 'sleeping' && (
        <span className="cube__zz">
          <i>z</i>
          <i>z</i>
          <i>z</i>
        </span>
      )}
    </div>
  )
}

function EyePair({ eyes, mood }: { eyes: Eyes; mood: Mood }) {
  const closed = mood === 'sleeping' || mood === 'stopped'
  const lid = mood === 'dreaming'
  const eyeClass = `cube__eye${lid ? ' cube__eye--lid' : ''}`
  return (
    <div className="cube__eyes">
      <span className={eyeClass}>
        <Eye shape={eyes} closed={closed} side="l" />
      </span>
      <span
        className={eyeClass}
        style={{ '--delay': '0.06s' } as React.CSSProperties}
      >
        <Eye shape={eyes} closed={closed} side="r" />
      </span>
    </div>
  )
}

function Eye({
  shape,
  closed,
  side,
}: {
  shape: Eyes
  closed: boolean
  side: 'l' | 'r'
}) {
  if (closed)
    return (
      <svg
        viewBox="0 0 40 40"
        fill="none"
        stroke="currentColor"
        strokeWidth="5"
        strokeLinecap="round"
      >
        <path d="M6 22c5 6 23 6 28 0" />
      </svg>
    )
  const glint = <circle cx="14" cy="14" r="4.2" fill="#fffdf9" />
  if (shape === 'wide')
    return (
      <svg viewBox="0 0 40 40" fill="currentColor">
        <circle cx="20" cy="20" r="18" />
        <circle cx="13" cy="13" r="5" fill="#fffdf9" />
      </svg>
    )
  if (shape === 'dot')
    return (
      <svg viewBox="0 0 40 40" fill="currentColor">
        <circle cx="20" cy="22" r="9" />
        <circle cx="17" cy="19" r="2.6" fill="#fffdf9" />
      </svg>
    )
  if (shape === 'sleepy')
    return (
      <svg viewBox="0 0 40 40" fill="currentColor">
        <path d="M3 20c0-6 8-10 17-10s17 4 17 10c-1 9-8 16-17 16S4 29 3 20z" />
        <circle cx="14" cy="18" r="3.4" fill="#fffdf9" />
      </svg>
    )
  return (
    <svg viewBox="0 0 40 40" fill="currentColor">
      {side === 'l' ? (
        <path d="M19 3c10-1 17 7 16 17-1 9-8 17-17 16C8 35 3 28 4 19 5 10 10 4 19 3z" />
      ) : (
        <path d="M21 4c9 0 16 8 15 17-1 10-9 16-18 15C9 35 3 28 4 19 5 10 12 4 21 4z" />
      )}
      {glint}
    </svg>
  )
}

function Wear({ accessory }: { accessory: Accessory }) {
  if (accessory === 'none') return null
  return (
    <svg className="cube__wear" viewBox="0 0 100 100" aria-hidden="true">
      {accessory === 'cap' && (
        <>
          <path d="M6 20 Q50 -4 94 20 L94 30 Q50 8 6 30z" fill="#2b2420" />
          <rect x="-8" y="25" width="60" height="9" rx="4.5" fill="#2b2420" />
        </>
      )}
      {accessory === 'glasses' && (
        <g fill="none" stroke="#2b2420" strokeWidth="4.5">
          <circle cx="33" cy="52" r="15.5" />
          <circle cx="67" cy="52" r="15.5" />
          <path d="M48.5 52h3" />
          <path d="M17.5 50 L4 44" strokeLinecap="round" />
          <path d="M82.5 50 L96 44" strokeLinecap="round" />
        </g>
      )}
      {accessory === 'bow' && (
        <g transform="translate(78 14)">
          <path
            d="M0 0 L-14 -9 L-14 9z M0 0 L14 -9 L14 9z"
            fill="#fffdf9"
            stroke="#2b2420"
            strokeWidth="3"
            strokeLinejoin="round"
          />
          <circle cx="0" cy="0" r="4" fill="#2b2420" />
        </g>
      )}
      {accessory === 'antenna' && (
        <g>
          <path
            d="M50 2 L50 -20"
            stroke="#2b2420"
            strokeWidth="4"
            strokeLinecap="round"
          />
          <circle
            cx="50"
            cy="-24"
            r="6"
            fill="var(--orange)"
            stroke="#2b2420"
            strokeWidth="3"
          />
        </g>
      )}
    </svg>
  )
}

const REST_X = -18
const REST_Y = 28
function useFollow(ref: React.RefObject<HTMLDivElement | null>, on: boolean) {
  useEffect(() => {
    const anchor = ref.current
    if (!on || !anchor) return
    if (matchMedia('(prefers-reduced-motion: reduce)').matches) return
    const body = anchor.querySelector<HTMLElement>('.cube__body')
    if (!body) return
    const state = { x: REST_X, y: REST_Y, tx: REST_X, ty: REST_Y, frame: 0 }
    const settle = () => {
      state.x += (state.tx - state.x) * 0.12
      state.y += (state.ty - state.y) * 0.12
      body.style.setProperty('--rx', `${state.x.toFixed(2)}deg`)
      body.style.setProperty('--ry', `${state.y.toFixed(2)}deg`)
      const moving =
        Math.abs(state.tx - state.x) > 0.05 ||
        Math.abs(state.ty - state.y) > 0.05
      state.frame = moving ? requestAnimationFrame(settle) : 0
    }
    const aim = (event: PointerEvent) => {
      const box = anchor.getBoundingClientRect()
      const clamp = (v: number) => Math.max(-1, Math.min(1, v))
      const dx = clamp((event.clientX - (box.left + box.width / 2)) / box.width)
      const dy = clamp(
        (event.clientY - (box.top + box.height / 2)) / box.height,
      )
      state.ty = REST_Y + dx * 16
      state.tx = REST_X - dy * 10
      anchor.style.setProperty('--look-x', `${(dx * 7).toFixed(1)}px`)
      anchor.style.setProperty('--look-y', `${(dy * 5).toFixed(1)}px`)
      if (state.frame === 0) state.frame = requestAnimationFrame(settle)
    }
    window.addEventListener('pointermove', aim, { passive: true })
    return () => {
      window.removeEventListener('pointermove', aim)
      cancelAnimationFrame(state.frame)
    }
  }, [ref, on])
}
