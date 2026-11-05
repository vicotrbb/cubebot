/** Handlers only mark the start: the card follows the pointer itself and notices it has left. */
type Hovered = {
  kind: 'agent' | 'task'
  id: string
  anchor: Element
  x: number
  y: number
  focused: boolean
}

let hovered: Hovered | null = null
const listeners = new Set<() => void>()

export const currentHover = () => hovered

export function setHover(next: Hovered | null) {
  hovered = next
  listeners.forEach((listener) => listener())
}

export function subscribeHover(listener: () => void) {
  listeners.add(listener)
  return () => {
    listeners.delete(listener)
  }
}

export const agentHover = (id: string) => hoverable('agent', id)

export const taskHover = (id: string) => hoverable('task', id)

function hoverable(kind: Hovered['kind'], id: string) {
  return {
    onPointerEnter: (event: React.PointerEvent) => {
      if (event.pointerType !== 'mouse') return
      setHover({
        kind,
        id,
        anchor: event.currentTarget,
        x: event.clientX,
        y: event.clientY,
        focused: false,
      })
    },
    onFocus: (event: React.FocusEvent) => {
      if (!event.target.matches(':focus-visible')) return
      const box = event.currentTarget.getBoundingClientRect()
      setHover({
        kind,
        id,
        anchor: event.currentTarget,
        x: box.right,
        y: box.top,
        focused: true,
      })
    },
    onBlur: () => {
      if (hovered?.focused) setHover(null)
    },
  }
}
