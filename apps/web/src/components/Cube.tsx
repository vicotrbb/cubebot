import { useEffect, useRef } from 'react'
import { agentHover } from './agentHover'
import { isDark } from './paint'
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
  'coral',
  'mint',
  'ocean',
  'charcoal',
] as const
export const EYES = [
  'round',
  'wide',
  'sleepy',
  'dot',
  'happy',
  'wink',
  'star',
  'heart',
] as const
export const ACCESSORIES = [
  'none',
  'cap',
  'glasses',
  'bow',
  'antenna',
  'crown',
  'headphones',
  'sprout',
  'party_hat',
  'monocle',
] as const
export type Paint = (typeof PAINTS)[number]
export type Eyes = (typeof EYES)[number]
export type Accessory = (typeof ACCESSORIES)[number]
export type Avatar = {
  paint: Paint
  eyes: Eyes
  accessory: Accessory
  /** A picked `#rrggbb` that replaces the paint. */
  color?: string | null
}
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

/** The paint classes and the custom color variable for a look; retired cubes are bare wood. */
export function paintOf(
  look: Avatar,
  retired = false,
): { classes: string; style: Record<string, string> } {
  if (retired) return { classes: 'cube--wood', style: {} }
  if (!look.color)
    return {
      classes: `cube--${look.paint}${look.paint === 'charcoal' ? ' cube--dark' : ''}`,
      style: {},
    }
  return {
    classes: `cube--custom${isDark(look.color) ? ' cube--dark' : ''}`,
    style: { '--custom': look.color },
  }
}

function cubeClasses(
  paint: string,
  mood: Mood,
  follow: boolean,
  className: string | undefined,
) {
  return ['cube', paint, `cube--${mood}`]
    .concat(follow ? 'cube--follow' : [], className ?? [])
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
  const paint = paintOf(look, mood === 'retired')
  const eyes = mood === 'retired' ? null : look.eyes
  const classes = cubeClasses(paint.classes, mood, follow, className)
  return (
    <div
      ref={ref}
      className={classes}
      style={{ '--size': `${size}px`, ...paint.style } as React.CSSProperties}
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

/** A flat front face showing one part of a look, for the avatar builder's choices. */
export function Face({
  look,
  part,
  size = 28,
}: {
  look: Avatar
  part: 'eyes' | 'accessory'
  size?: number
}) {
  const paint = paintOf(look)
  return (
    <span
      className={`cube cube-face ${paint.classes}`}
      style={{ '--size': `${size}px`, ...paint.style } as React.CSSProperties}
      aria-hidden="true"
    >
      <span className="cube__face">
        <EyePair eyes={look.eyes} mood="idle" />
        {part === 'accessory' && <Wear accessory={look.accessory} />}
      </span>
    </span>
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
  if (closed || (shape === 'wink' && side === 'r') || shape === 'happy')
    return closed ? <Lid /> : <Smile />
  const glint = <circle className="cube__glint" cx="14" cy="14" r="4.2" />
  if (shape === 'star')
    return (
      <svg viewBox="0 0 40 40" fill="currentColor">
        <path d="M20 2l5 11.6 12.4 1.1-9.4 8.3 2.8 12.2L20 28.8 9.2 35.2 12 23 2.6 14.7 15 13.6z" />
        <circle className="cube__glint" cx="16" cy="15" r="2.8" />
      </svg>
    )
  if (shape === 'heart')
    return (
      <svg viewBox="0 0 40 40">
        <path
          d="M20 35C7 26 2 18 5.5 11.2 8.7 5 16.4 5.2 20 11c3.6-5.8 11.3-6 14.5.2C38 18 33 26 20 35z"
          fill="#d64561"
        />
        <circle cx="13" cy="13" r="3.2" fill="#fffdf9" />
      </svg>
    )

  if (shape === 'wide')
    return (
      <svg viewBox="0 0 40 40" fill="currentColor">
        <circle cx="20" cy="20" r="18" />
        <circle className="cube__glint" cx="13" cy="13" r="5" />
      </svg>
    )
  if (shape === 'dot')
    return (
      <svg viewBox="0 0 40 40" fill="currentColor">
        <circle cx="20" cy="22" r="9" />
        <circle className="cube__glint" cx="17" cy="19" r="2.6" />
      </svg>
    )
  if (shape === 'sleepy')
    return (
      <svg viewBox="0 0 40 40" fill="currentColor">
        <path d="M3 20c0-6 8-10 17-10s17 4 17 10c-1 9-8 16-17 16S4 29 3 20z" />
        <circle className="cube__glint" cx="14" cy="18" r="3.4" />
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

function Lid() {
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
}

/** A happy closed eye, the arc turned up. */
function Smile() {
  return (
    <svg
      viewBox="0 0 40 40"
      fill="none"
      stroke="currentColor"
      strokeWidth="5"
      strokeLinecap="round"
    >
      <path d="M6 26c5-10 23-10 28 0" />
    </svg>
  )
}

function Wear({ accessory }: { accessory: Accessory }) {
  if (accessory === 'none') return null
  return (
    <svg className="cube__wear" viewBox="0 0 100 100" aria-hidden="true">
      {accessory === 'cap' && (
        <>
          <path
            d="M6 20 Q50 -4 94 20 L94 30 Q50 8 6 30z"
            fill="var(--wear, #2b2420)"
          />
          <rect
            x="-8"
            y="25"
            width="60"
            height="9"
            rx="4.5"
            fill="var(--wear, #2b2420)"
          />
        </>
      )}
      {accessory === 'glasses' && (
        <g fill="none" stroke="var(--wear, #2b2420)" strokeWidth="4.5">
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
      {accessory === 'crown' && (
        <g strokeLinejoin="round">
          <path
            d="M28 4 L28 -18 L39 -7 L50 -24 L61 -7 L72 -18 L72 4z"
            fill="#f0cf5a"
            stroke="#2b2420"
            strokeWidth="3.5"
          />
          <circle
            cx="50"
            cy="-4"
            r="4"
            fill="#e5a0b4"
            stroke="#2b2420"
            strokeWidth="2"
          />
        </g>
      )}
      {accessory === 'headphones' && (
        <g stroke="#2b2420" strokeWidth="3.5" strokeLinejoin="round">
          <path
            d="M6 44 C6 -12 94 -12 94 44"
            fill="none"
            stroke="var(--wear, #2b2420)"
            strokeWidth="7"
          />
          <rect
            x="-6"
            y="34"
            width="17"
            height="30"
            rx="7"
            fill="var(--orange)"
          />
          <rect
            x="89"
            y="34"
            width="17"
            height="30"
            rx="7"
            fill="var(--orange)"
          />
        </g>
      )}
      {accessory === 'sprout' && (
        <g stroke="#2b2420" strokeWidth="3" strokeLinejoin="round">
          <path
            d="M50 2 C50 -6 51 -12 50 -18"
            fill="none"
            strokeLinecap="round"
          />
          <path
            d="M50 -14 C42 -28 26 -24 27 -13 C35 -7 45 -9 50 -14z"
            fill="#9bb7a4"
          />
          <path
            d="M50 -18 C56 -32 74 -30 72 -19 C65 -12 55 -13 50 -18z"
            fill="#b4cbbb"
          />
        </g>
      )}
      {accessory === 'party_hat' && (
        <g
          transform="translate(70 4) rotate(16)"
          stroke="#2b2420"
          strokeWidth="3"
          strokeLinejoin="round"
        >
          <path d="M-15 0 L0 -40 L15 0z" fill="#e5a0b4" />
          <path
            d="M-9 -15 L7 -9 M-4 -28 L4 -25"
            fill="none"
            stroke="#fffdf9"
            strokeWidth="3.5"
          />
          <circle cx="0" cy="-43" r="5.5" fill="#f0cf5a" />
        </g>
      )}
      {accessory === 'monocle' && (
        <g fill="none" stroke="var(--wear, #2b2420)">
          <circle
            cx="67"
            cy="52"
            r="16"
            strokeWidth="4.5"
            fill="rgb(255 255 255 / 0.18)"
          />
          <path
            d="M80 62 C84 76 90 82 98 84"
            strokeWidth="2.2"
            strokeLinecap="round"
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
