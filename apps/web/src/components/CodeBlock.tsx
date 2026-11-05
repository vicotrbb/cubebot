import { useEffect, useState, type ReactNode } from 'react'
import { CopyButton } from './primitives'

/** Past this, highlighting would hold the page up; the code still shows, plain. */
const HIGHLIGHT_BYTES = 512 * 1024

const library = () => import('./highlight')

function useHighlight(code: string, language: string | undefined) {
  const [html, setHtml] = useState<{ code: string; html: string } | null>(null)
  useEffect(() => {
    if (!language || code.length > HIGHLIGHT_BYTES) return
    let current = true
    void library().then(({ highlighted }) => {
      const made = highlighted(code, language)
      if (current && made !== null) setHtml({ code, html: made })
    })
    return () => {
      current = false
    }
  }, [code, language])
  // A stale result never shows over newer text, as while a reply streams in.
  return html?.code === code ? html.html : null
}

export function CodeBlock({
  code,
  language,
  name,
  numbers = false,
  bare = false,
  maxHeight,
  actions,
}: {
  code: string
  language?: string | undefined
  name?: string | undefined
  numbers?: boolean
  bare?: boolean
  maxHeight?: number
  actions?: ReactNode
}) {
  const html = useHighlight(code, language)
  const text = code.endsWith('\n') ? code.slice(0, -1) : code
  const lines = text.split('\n').length
  return (
    <div className={bare ? 'codeblock codeblock--bare' : 'codeblock'}>
      {!bare && (
        <div className="codeblock__head">
          <span className="codeblock__name">{name ?? language ?? 'text'}</span>
          {actions}
          <CopyButton text={code} label="Copy code" />
        </div>
      )}
      <div
        className="codeblock__scroll"
        tabIndex={0}
        style={maxHeight ? { maxHeight } : undefined}
      >
        {numbers && (
          <span className="codeblock__gutter" aria-hidden="true">
            {Array.from({ length: lines }, (_, index) => index + 1).join('\n')}
          </span>
        )}
        <pre>
          {html === null ? (
            <code>{text}</code>
          ) : (
            // highlight.js escapes the code; the only markup in this is its own spans.
            <code dangerouslySetInnerHTML={{ __html: html }} />
          )}
        </pre>
      </div>
    </div>
  )
}
