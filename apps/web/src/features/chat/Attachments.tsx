import { FileText } from 'lucide-react'
import { useState } from 'react'
import type { Turn } from '../../api/client'
import { Alert } from '../../components/primitives'
import { FileViewer, useFileBlob, type Viewable } from '../files/FileViewer'
import { fileSize, kindOf, shownName } from '../files/viewer'
import './markdown.css'

type Attachment = Turn['attachments'][number]

/** Media loads by itself below this; a larger file waits behind its chip. */
const INLINE_BYTES = 64 * 1024 * 1024

/** What an agent sent with a reply: pictures inline, audio and video with a player, the rest as a
 *  chip. Anything opens in the viewer. */
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
    if (kind === 'image') return <Picture file={file} onOpen={onOpen} />
    if (kind === 'audio' || kind === 'video')
      return <Player file={file} kind={kind} />
  }
  return (
    <button type="button" className="attachment-file" onClick={onOpen}>
      <FileText aria-hidden="true" />
      <span>{file.name}</span>
      <small>{fileSize(file.size)}</small>
    </button>
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
