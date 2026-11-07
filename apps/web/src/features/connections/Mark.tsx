import { Plug } from 'lucide-react'
import { logos } from '../../../../../modules/brand/logos'

/** The connector's own brand mark, unmodified; a plug only for a connector without one. */
export function Mark({ id, size = 'md' }: { id: string; size?: 'md' | 'lg' }) {
  const logo = logos[id]
  return (
    <span className={`mark mark--${size}`} aria-hidden="true">
      {logo ? (
        // Static marks from our own source, never user content.
        <svg
          viewBox={logo.viewBox}
          dangerouslySetInnerHTML={{ __html: logo.svg }}
        />
      ) : (
        <Plug />
      )}
    </span>
  )
}
