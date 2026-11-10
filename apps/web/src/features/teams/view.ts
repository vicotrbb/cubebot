export type View = { k: number; x: number; y: number }
export type Point = { x: number; y: number }

export const PAD = 24
export const MAX = 2

export function fitted(
  frame: { w: number; h: number },
  width: number,
  height: number,
): View {
  const room = (side: number) => Math.max(1, side - PAD * 2)
  const k = Math.min(1, room(frame.w) / width, room(frame.h) / height)
  return {
    k,
    x: (frame.w - width * k) / 2,
    y: Math.max(PAD, (frame.h - height * k) / 2),
  }
}

export function zoomed(view: View, factor: number, at: Point, min: number) {
  const k = Math.min(MAX, Math.max(min, view.k * factor))
  const ratio = k / view.k
  return {
    k,
    x: at.x - (at.x - view.x) * ratio,
    y: at.y - (at.y - view.y) * ratio,
  }
}
