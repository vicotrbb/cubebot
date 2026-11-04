import { Download, X } from 'lucide-react'
import { useEffect, useRef, useState, type ReactNode } from 'react'
import { CodeBlock } from '../../components/CodeBlock'
import { Diagram } from '../../components/Diagram'
import { Frame } from '../../components/Frame'
import { framed, framedSvg } from '../../components/framePage'
import { Alert, Skeleton } from '../../components/primitives'
import { Markdown } from '../chat/Markdown'
import { ViewSwitch } from '../chat/Preview'
import '../chat/chat.css'
import { PdfPages } from './PdfPages'
import { TablePreview } from './TablePreview'
import {
  blobType,
  fileSize,
  grammarOf,
  kindOf,
  looksLikeText,
  type Kind,
} from './viewer'
import './viewer.css'

export type Viewable = {
  name: string
  content_type?: string | null | undefined
  size: number
  url: string
}

/** Past this a preview would hold too much in memory; the download always works. */
const MEDIA_BYTES = 512 * 1024 * 1024
const TEXT_BYTES = 2 * 1024 * 1024

/** The file's bytes as a blob and its URL. The server always answers with a download, which is
 *  what keeps a file from ever rendering in the console's origin; reading it here shows it safely. */
export function useFileBlob(file: Viewable) {
  const [loaded, setLoaded] = useState<{ blob: Blob; url: string } | null>(null)
  const [failed, setFailed] = useState(false)
  useEffect(() => {
    const abort = new AbortController()
    let url = ''
    fetch(file.url, { credentials: 'same-origin', signal: abort.signal })
      .then((response) => {
        if (!response.ok) throw new Error(String(response.status))
        return response.arrayBuffer()
      })
      .then((bytes) => {
        const type = blobType(file.name, file.content_type)
        const blob = new Blob([bytes], { type })
        url = URL.createObjectURL(blob)
        setLoaded({ blob, url })
      })
      .catch(() => {
        if (!abort.signal.aborted) setFailed(true)
      })
    return () => {
      abort.abort()
      if (url) URL.revokeObjectURL(url)
    }
  }, [file.url, file.name, file.content_type])
  return { blob: loaded?.blob ?? null, url: loaded?.url ?? '', failed }
}

/** Opens a file over the page: full screen on a phone, a large sheet on a desk. */
export function FileViewer({
  file,
  onClose,
}: {
  file: Viewable
  onClose: () => void
}) {
  const ref = useRef<HTMLDialogElement>(null)
  useEffect(() => {
    const dialog = ref.current
    if (dialog && !dialog.open) dialog.showModal()
  }, [])
  const kind = kindOf(file.name, file.content_type)
  const tooBig =
    file.size >
    (['image', 'audio', 'video', 'pdf'].includes(kind ?? '')
      ? MEDIA_BYTES
      : Infinity)
  return (
    <dialog
      ref={ref}
      className="viewer"
      aria-label={file.name}
      onClose={onClose}
      onClick={(event) => {
        if (event.target === event.currentTarget) onClose()
      }}
    >
      <div className="viewer__sheet">
        <header className="viewer__head">
          <div className="viewer__title">
            <h2>{file.name}</h2>
            <span>{fileSize(file.size)}</span>
          </div>
          <a
            className="btn btn--sm"
            href={file.url}
            download={file.name}
            aria-label={`Download ${file.name}`}
          >
            <Download />
            <span className="viewer__word">Download</span>
          </a>
          <button
            type="button"
            className="btn btn--ghost btn--icon"
            aria-label="Close"
            onClick={onClose}
          >
            <X />
          </button>
        </header>
        <div className="viewer__body" tabIndex={0}>
          {tooBig ? (
            <Unavailable file={file} big />
          ) : kind ? (
            <Preview file={file} kind={kind} />
          ) : (
            <Unknown file={file} />
          )}
        </div>
      </div>
    </dialog>
  )
}

function Unavailable({ file, big }: { file: Viewable; big: boolean }) {
  return (
    <div className="viewer__none">
      <h3>
        {big
          ? 'This file is too large to open here.'
          : 'Cubebot cannot show this kind of file.'}
      </h3>
      <p>Download {file.name} to open it on your device.</p>
      <a className="btn btn--primary" href={file.url} download={file.name}>
        <Download /> Download
      </a>
    </div>
  )
}

/** A name and type that say nothing: it still shows if its bytes read as text. */
function Unknown({ file }: { file: Viewable }) {
  const [text, setText] = useState<boolean | null>(
    file.size > TEXT_BYTES ? false : null,
  )
  useEffect(() => {
    if (text !== null) return
    const abort = new AbortController()
    fetch(file.url, { credentials: 'same-origin', signal: abort.signal })
      .then((response) => response.arrayBuffer())
      .then((bytes) => setText(looksLikeText(new Uint8Array(bytes))))
      .catch(() => {
        if (!abort.signal.aborted) setText(false)
      })
    return () => abort.abort()
  }, [file.url, text])
  if (text === null)
    return (
      <div className="viewer__wait" role="status" aria-label="Loading file">
        <Skeleton lines={5} />
      </div>
    )
  return text ? (
    <Preview file={file} kind="text" />
  ) : (
    <Unavailable file={file} big={false} />
  )
}

function Preview({ file, kind }: { file: Viewable; kind: NonNullable<Kind> }) {
  const { blob, url, failed } = useFileBlob(file)
  if (failed) return <Alert>This file could not be loaded. Try again.</Alert>
  if (!blob)
    return (
      <div className="viewer__wait" role="status" aria-label="Loading file">
        <Skeleton lines={5} />
      </div>
    )
  if (kind === 'pdf') return <PdfPages blob={blob} />
  if (kind === 'image' && /\.svg$/i.test(file.name))
    return <TextPreview blob={blob} kind="svg" name={file.name} />
  if (kind === 'image' || kind === 'audio' || kind === 'video')
    return <Media url={url} kind={kind} name={file.name} />
  return <TextPreview blob={blob} kind={kind} name={file.name} />
}

function Media({
  url,
  kind,
  name,
}: {
  url: string
  kind: 'image' | 'audio' | 'video'
  name: string
}) {
  if (kind === 'image')
    return (
      <div className="viewer__picture">
        <img src={url} alt={name} />
      </div>
    )
  if (kind === 'audio')
    return (
      <div className="viewer__listen">
        <audio controls src={url} aria-label={name} />
      </div>
    )
  return (
    <div className="viewer__picture">
      <video controls playsInline src={url} aria-label={name} />
    </div>
  )
}

function TextPreview({
  blob,
  kind,
  name,
}: {
  blob: Blob
  kind: string
  name: string
}) {
  const [text, setText] = useState<string | null>(null)
  const cut = blob.size > TEXT_BYTES
  useEffect(() => {
    let current = true
    void blob
      .slice(0, TEXT_BYTES)
      .text()
      .then((read) => {
        if (current) setText(read)
      })
    return () => {
      current = false
    }
  }, [blob])
  if (text === null) return null
  return (
    <>
      {cut && (
        <p className="viewer__cut">
          Showing the first 2 MiB. Download the file for all of it.
        </p>
      )}
      <Shown kind={kind} text={text} name={name} />
    </>
  )
}

/** What each drawable kind becomes; the rest is simply code. */
function drawing(kind: string, text: string, name: string): ReactNode {
  if (kind === 'markdown')
    return (
      <div className="viewer__page">
        <Markdown text={text} />
      </div>
    )
  if (kind === 'csv') return <TablePreview text={text} />
  if (kind === 'tsv') return <TablePreview text={text} separator={'\t'} />
  if (kind === 'html')
    return <Frame fill page={framed(text)} title={`${name}, previewed`} />
  if (kind === 'svg')
    return <Frame fill page={framedSvg(text)} title={`${name}, previewed`} />
  if (kind === 'mermaid')
    return (
      <div className="viewer__page">
        <Diagram source={text} />
      </div>
    )
  return null
}

function Shown({
  kind,
  text,
  name,
}: {
  kind: string
  text: string
  name: string
}) {
  const [code, setCode] = useState(false)
  const drawn = drawing(kind, text, name)
  const source = (
    <CodeBlock
      bare
      numbers
      code={kind === 'json' ? pretty(text) : text}
      language={grammarOf(name)}
    />
  )
  if (!drawn) return source
  return (
    <>
      <div className="viewer__views">
        <ViewSwitch code={code} onChange={setCode} />
      </div>
      {code ? source : drawn}
    </>
  )
}

function pretty(text: string) {
  try {
    return JSON.stringify(JSON.parse(text), null, 2)
  } catch {
    return text
  }
}
