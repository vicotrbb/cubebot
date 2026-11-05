import { type RefObject, useEffect } from 'react'

/** Diagrams, images and code arrive after the first paint; a pinned transcript stays on the latest message. */
export function useFollowEnd(
  scroller: RefObject<HTMLElement | null>,
  pinned: RefObject<boolean>,
) {
  useEffect(() => {
    const element = scroller.current
    if (!element) return
    const sizes = new ResizeObserver(() => {
      if (pinned.current) element.scrollTop = element.scrollHeight
    })
    const watch = () => {
      sizes.disconnect()
      sizes.observe(element)
      for (const child of element.children) sizes.observe(child)
    }
    const added = new MutationObserver(watch)
    added.observe(element, { childList: true })
    watch()
    return () => {
      added.disconnect()
      sizes.disconnect()
    }
  }, [scroller, pinned])
}
