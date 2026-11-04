import katex from 'katex'
import 'katex/dist/katex.min.css'

/** KaTeX without `trust` emits no links or scripts, and a mistake shows as red source. */
export default function Math({
  text,
  block,
}: {
  text: string
  block: boolean
}) {
  const html = katex.renderToString(text, {
    displayMode: block,
    throwOnError: false,
  })
  const Tag = block ? 'div' : 'span'
  return (
    <Tag
      className={block ? 'md__math' : undefined}
      dangerouslySetInnerHTML={{ __html: html }}
    />
  )
}
