/** Which agent the pointer or keyboard is on, for the one card the shell draws. Handlers only
 *  mark the start: the card follows the pointer itself and notices it has left. */
export type Hovered = {
  id: string
  anchor: Element
  /** Where the card goes first; the pointer moves it from there. */
  x: number
  y: number
  /** Opened by the keyboard, so no pointer will move or close it. */
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

/** What goes on anything that stands for an agent. Touch has no hover, so it is left alone. */
export function agentHover(id: string) {
  return {
    onPointerEnter: (event: React.PointerEvent) => {
      if (event.pointerType !== 'mouse') return
      setHover({
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
