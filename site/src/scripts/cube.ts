// The hero block turns toward the pointer. Rotation targets are CSS custom
// properties on the block body so the stylesheet keeps the resting pose.

const REST_X = -18
const REST_Y = 28
const RANGE_X = 10
const RANGE_Y = 16
const LOOK_X = 7
const LOOK_Y = 5

interface Follower {
  body: HTMLElement
  anchor: HTMLElement
  x: number
  y: number
  targetX: number
  targetY: number
  frame: number
}

function clamp(value: number): number {
  return Math.max(-1, Math.min(1, value))
}

function aim(follower: Follower, pointerX: number, pointerY: number): void {
  const box = follower.anchor.getBoundingClientRect()
  const dx = clamp((pointerX - (box.left + box.width / 2)) / box.width)
  const dy = clamp((pointerY - (box.top + box.height / 2)) / box.height)
  follower.targetY = REST_Y + dx * RANGE_Y
  follower.targetX = REST_X - dy * RANGE_X
  follower.body.style.setProperty('--look-x', `${(dx * LOOK_X).toFixed(1)}px`)
  follower.body.style.setProperty('--look-y', `${(dy * LOOK_Y).toFixed(1)}px`)
  if (follower.frame === 0) {
    follower.frame = requestAnimationFrame(() => settle(follower))
  }
}

function settle(follower: Follower): void {
  follower.x += (follower.targetX - follower.x) * 0.12
  follower.y += (follower.targetY - follower.y) * 0.12
  follower.body.style.setProperty('--rx', `${follower.x.toFixed(2)}deg`)
  follower.body.style.setProperty('--ry', `${follower.y.toFixed(2)}deg`)
  const moving =
    Math.abs(follower.targetX - follower.x) > 0.05 ||
    Math.abs(follower.targetY - follower.y) > 0.05
  follower.frame = moving ? requestAnimationFrame(() => settle(follower)) : 0
}

function attach(anchor: HTMLElement): void {
  const body = anchor.querySelector<HTMLElement>('.cube__body')
  if (!body) return
  const follower: Follower = {
    body,
    anchor,
    x: REST_X,
    y: REST_Y,
    targetX: REST_X,
    targetY: REST_Y,
    frame: 0,
  }
  window.addEventListener(
    'pointermove',
    (event) => aim(follower, event.clientX, event.clientY),
    { passive: true },
  )
}

const reduced = window.matchMedia('(prefers-reduced-motion: reduce)').matches
if (!reduced) {
  document.querySelectorAll<HTMLElement>('[data-follow]').forEach(attach)
}
