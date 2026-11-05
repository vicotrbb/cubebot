import { useEffect, useId, useState } from 'react'

const mermaid = () => import('mermaid').then((module) => module.default)

function dark() {
  const theme = document.documentElement.dataset.theme
  return theme
    ? theme === 'dark'
    : matchMedia('(prefers-color-scheme: dark)').matches
}

/** Agents and people write the source, so scripts in it stay inert. */
function useDiagram(source: string) {
  const id = useId().replaceAll(/[^a-zA-Z0-9]/g, '')
  const [drawn, setDrawn] = useState<{ svg?: string; error?: string }>({})
  useEffect(() => {
    let current = true
    const draw = window.setTimeout(async () => {
      try {
        const library = await mermaid()
        library.initialize({
          startOnLoad: false,
          securityLevel: 'strict',
          suppressErrorRendering: true,
          theme: dark() ? 'dark' : 'neutral',
          fontFamily: 'Rubik, system-ui, sans-serif',
        })
        const { svg } = await library.render(`diagram${id}`, source)
        if (current) setDrawn({ svg })
      } catch (error) {
        if (current) setDrawn({ error: String(error).split('\n')[0] ?? '' })
      }
    }, 300)
    return () => {
      current = false
      window.clearTimeout(draw)
    }
  }, [id, source])
  return drawn
}

export function Diagram({ source }: { source: string }) {
  const { svg, error } = useDiagram(source)
  if (error)
    return (
      <p className="kb-diagram__error small" contentEditable={false}>
        This diagram has a mistake: {error}
      </p>
    )
  if (!svg)
    return <div className="skeleton kb-diagram__wait" contentEditable={false} />
  return (
    <div
      className="kb-diagram__art"
      contentEditable={false}
      // Rendered by mermaid in strict mode, which sanitizes what it emits.
      dangerouslySetInnerHTML={{ __html: svg }}
    />
  )
}
