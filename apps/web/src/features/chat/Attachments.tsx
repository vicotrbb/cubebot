import { FileText } from 'lucide-react'
import { type ReactNode, useEffect, useRef, useState } from 'react'
import type { Turn } from '../../api/client'
import { Alert } from '../../components/primitives'
import { FileViewer, useFileBlob, type Viewable } from '../files/FileViewer'
import { fileSize, kindOf, shownName } from '../files/viewer'
import './markdown.css'

type Attachment = Turn['attachments'][number]

const INLINE_BYTES = 64 * 1024 * 1024

export function Attachments({
  turn,
  items,
}: {
  turn: string
  items: Attachment[]
}) {
  const [open, setOpen] = useState<Viewable | null>(null)
  if (items.length === 0) return null
  return (
    <div className="attachments">
      {items.map((item) => {
        const file: Viewable = {
          name: shownName(item.name),
          content_type: item.content_type,
          size: item.size,
          url: `/api/attachments/${turn}/${item.file}/content`,
        }
        return <Item key={item.file} file={file} onOpen={() => setOpen(file)} />
      })}
      {open && <FileViewer file={open} onClose={() => setOpen(null)} />}
    </div>
  )
}

function Item({ file, onOpen }: { file: Viewable; onOpen: () => void }) {
  const kind = kindOf(file.name, file.content_type)
  if (file.size <= INLINE_BYTES) {
    if (kind === 'image')
      return (
        <Near>
          <Picture file={file} onOpen={onOpen} />
        </Near>
      )
    if (kind === 'audio' || kind === 'video')
      return (
        <Near>
          <Player file={file} kind={kind} />
        </Near>
      )
  }
  return (
    <button type="button" className="attachment-file" onClick={onOpen}>
      <FileText aria-hidden="true" />
      <span>{file.name}</span>
      <small>{fileSize(file.size)}</small>
    </button>
  )
}

function Near({ children }: { children: ReactNode }) {
  const [near, setNear] = useState(false)
  const spot = useRef<HTMLSpanElement>(null)
  useEffect(() => {
    const element = spot.current
    if (!element) return
    const watch = new IntersectionObserver(
      ([entry]) => {
        if (entry?.isIntersecting) setNear(true)
      },
      { root: element.closest('.transcript'), rootMargin: '600px' },
    )
    watch.observe(element)
    return () => watch.disconnect()
  }, [near])
  if (near) return children
  return (
    <span ref={spot} className="attachment-image">
      <span className="skeleton" aria-hidden="true" />
    </span>
  )
}

function Picture({ file, onOpen }: { file: Viewable; onOpen: () => void }) {
  const { url, failed } = useFileBlob(file)
  if (failed) return <Alert>{file.name} could not be loaded.</Alert>
  return (
    <button
      type="button"
      className="attachment-image"
      aria-label={`Open ${file.name}`}
      onClick={onOpen}
    >
      {url ? (
        <img src={url} alt={file.name} />
      ) : (
        <span className="skeleton" aria-hidden="true" />
      )}
    </button>
  )
}

function Player({ file, kind }: { file: Viewable; kind: 'audio' | 'video' }) {
  const { url, failed } = useFileBlob(file)
  if (failed) return <Alert>{file.name} could not be loaded.</Alert>
  return (
    <figure className="attachment-media">
      {url &&
        (kind === 'audio' ? (
          <audio controls src={url} aria-label={file.name} />
        ) : (
          <video controls playsInline src={url} aria-label={file.name} />
        ))}
      <figcaption>{file.name}</figcaption>
    </figure>
  )
}
