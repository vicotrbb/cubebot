import { NodeViewWrapper, type NodeViewProps } from '@tiptap/react'
import { ImageOff } from 'lucide-react'
import { useEffect, useState } from 'react'
import { safeHref } from '../chat/tokens'

export function ImageView({ node, selected }: NodeViewProps) {
  const { src, alt, title } = node.attrs as {
    src: string
    alt: string | null
    title: string | null
  }
  const [failed, setFailed] = useState(false)
  useEffect(() => setFailed(false), [src])
  const href = safeHref(src)
  return (
    <NodeViewWrapper
      className="kb-image"
      data-selected={selected}
      data-drag-handle
    >
      {failed ? (
        <span className="kb-image__missing" contentEditable={false}>
          <ImageOff aria-hidden />
          <span>{alt || 'Image'} could not be loaded.</span>
          {href && (
            <a href={href} target="_blank" rel="noopener noreferrer">
              Open the original
            </a>
          )}
        </span>
      ) : (
        <img
          src={src}
          alt={alt ?? ''}
          title={title ?? undefined}
          loading="lazy"
          referrerPolicy="no-referrer"
          onError={() => setFailed(true)}
        />
      )}
    </NodeViewWrapper>
  )
}
