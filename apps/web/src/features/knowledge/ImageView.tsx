import { NodeViewWrapper, type NodeViewProps } from '@tiptap/react'
import { ImageOff } from 'lucide-react'
import { useState } from 'react'
import { inlineImage, safeHref } from '../chat/tokens'

export function ImageView({ node, selected }: NodeViewProps) {
  const { src, alt, title } = node.attrs as {
    src: string
    alt: string | null
    title: string | null
  }
  const [failedSrc, setFailedSrc] = useState<string | null>(null)
  const href = safeHref(src)
  const shown = inlineImage(src)
  return (
    <NodeViewWrapper
      className="kb-image"
      data-selected={selected}
      data-drag-handle
    >
      {!shown || failedSrc === src ? (
        <span className="kb-image__missing" contentEditable={false}>
          <ImageOff aria-hidden />
          <span>
            {alt || 'Image'}{' '}
            {shown
              ? 'could not be loaded.'
              : 'is hosted elsewhere, so it is not loaded with the page.'}
          </span>
          {href && (
            <a href={href} target="_blank" rel="noopener noreferrer">
              Open the original
            </a>
          )}
        </span>
      ) : (
        <img
          src={shown}
          alt={alt ?? ''}
          title={title ?? undefined}
          loading="lazy"
          referrerPolicy="no-referrer"
          onError={() => setFailedSrc(src)}
        />
      )}
    </NodeViewWrapper>
  )
}
