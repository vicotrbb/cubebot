import { useEffect, useRef, useState } from 'react'

const MIN = 120
const MAX = 640

/** Runs a page where it cannot reach the console: an opaque origin with no network. */
export function Frame({
  page,
  title,
  fill = false,
}: {
  page: string
  title: string
  fill?: boolean
}) {
  const ref = useRef<HTMLIFrameElement>(null)
  const [height, setHeight] = useState(MIN)
  useEffect(() => {
    if (fill) return
    const measure = (event: MessageEvent<{ frameHeight?: unknown }>) => {
      if (event.source !== ref.current?.contentWindow) return
      const reported = Number(event.data?.frameHeight)
      if (Number.isFinite(reported))
        setHeight(Math.min(MAX, Math.max(MIN, reported)))
    }
    window.addEventListener('message', measure)
    return () => window.removeEventListener('message', measure)
  }, [fill])
  return (
    <iframe
      ref={ref}
      className={fill ? 'viewer__frame' : 'preview__frame'}
      style={fill ? undefined : { height }}
      title={title}
      sandbox="allow-scripts"
      srcDoc={page}
    />
  )
}
