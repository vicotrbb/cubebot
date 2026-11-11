import { FileText, LoaderCircle, RotateCw, X } from 'lucide-react'
import { useEffect, useRef, useState, type DragEvent } from 'react'
import { api, failure, ok } from '../../api/client'
import { useSession } from '../../app/queries'
import type { components } from '../../api/schema'
import { NameTaken, uploadFile } from '../files/upload'
import { fileSize } from '../files/viewer'

type FileEntry = components['schemas']['FileEntry']

export const MOST_FILES = 10
const FOLDER = 'Chat uploads'
const folders = new Map<string, Promise<string>>()

type Pending = {
  key: string
  file: File
  preview: string | null
  entry?: FileEntry | undefined
  error?: string | undefined
}

async function find() {
  const root = await ok(api.GET('/api/files', { params: { query: {} } }))
  return root.find((entry) => entry.folder && entry.name === FOLDER)?.id
}

async function make() {
  const found = await find()
  if (found) return found
  try {
    const body = { parent: null, name: FOLDER }
    return (await ok(api.POST('/api/files/folders', { body }))).id
  } catch (error) {
    const raced = await find()
    if (raced) return raced
    throw error
  }
}

function folder(account: string) {
  const known = folders.get(account)
  if (known) return known
  const made = make()
  folders.set(account, made)
  made.catch(() => folders.delete(account))
  return made
}

function renamed(name: string) {
  const dot = name.lastIndexOf('.')
  const [stem, extension] =
    dot > 0 ? [name.slice(0, dot), name.slice(dot)] : [name, '']
  return `${stem} ${crypto.randomUUID().slice(0, 6)}${extension}`
}

async function upload(account: string, file: File) {
  const parent = await folder(account)
  try {
    return await uploadFile(file, parent)
  } catch (error) {
    if (error instanceof NameTaken)
      return uploadFile(file, parent, renamed(file.name))
    folders.delete(account)
    throw error
  }
}

export function useAttachments(onChange: () => void) {
  const account = useSession().data?.id ?? ''
  const [items, setItems] = useState<Pending[]>([])
  const [error, setError] = useState('')
  const shown = useRef(items)
  useEffect(() => {
    shown.current = items
  }, [items])
  useEffect(
    () => () =>
      shown.current.forEach(
        (item) => item.preview && URL.revokeObjectURL(item.preview),
      ),
    [],
  )
  function update(key: string, change: Partial<Pending>) {
    setItems((current) =>
      current.map((item) => (item.key === key ? { ...item, ...change } : item)),
    )
  }
  function start(item: Pending) {
    upload(account, item.file).then(
      (entry) => update(item.key, { entry }),
      (problem: unknown) => update(item.key, { error: failure(problem) }),
    )
  }
  function add(files: File[]) {
    const room = MOST_FILES - shown.current.length
    setError(
      files.length > room
        ? `A message carries at most ${MOST_FILES} files.`
        : '',
    )
    const fresh = files.slice(0, Math.max(room, 0)).map((file) => ({
      key: crypto.randomUUID(),
      file,
      preview: file.type.startsWith('image/')
        ? URL.createObjectURL(file)
        : null,
    }))
    if (fresh.length === 0) return
    setItems((current) => [...current, ...fresh])
    fresh.forEach(start)
    onChange()
  }
  function remove(key: string) {
    const gone = shown.current.find((item) => item.key === key)
    if (gone?.preview) URL.revokeObjectURL(gone.preview)
    setItems((current) => current.filter((item) => item.key !== key))
    setError('')
    onChange()
  }
  function retry(item: Pending) {
    update(item.key, { error: undefined })
    start(item)
  }
  function clear() {
    shown.current.forEach(
      (item) => item.preview && URL.revokeObjectURL(item.preview),
    )
    setItems([])
    setError('')
  }
  return {
    items,
    error,
    add,
    remove,
    retry,
    clear,
    files: items.flatMap((item) => (item.entry ? [item.entry.id] : [])),
    blocked: items.some((item) => !item.entry),
  }
}

export type Attachments = ReturnType<typeof useAttachments>

export function Tray({ attachments }: { attachments: Attachments }) {
  if (attachments.items.length === 0) return null
  return (
    <ul className="composer-files" aria-label="Files to send">
      {attachments.items.map((item) => (
        <Tile key={item.key} item={item} attachments={attachments} />
      ))}
    </ul>
  )
}

function Tile({
  item,
  attachments,
}: {
  item: Pending
  attachments: Attachments
}) {
  const name = item.file.name
  const state = item.error ? 'failed' : item.entry ? 'ready' : 'uploading'
  return (
    <li
      className={`composer-file composer-file--${state}${item.preview ? ' composer-file--image' : ''}`}
      title={item.error ?? name}
    >
      {item.preview ? (
        <img src={item.preview} alt={name} />
      ) : (
        <>
          <FileText className="composer-file__icon" aria-hidden="true" />
          <span className="composer-file__text">
            <span className="composer-file__name">{name}</span>
            <small>{fileSize(item.file.size)}</small>
          </span>
        </>
      )}
      {state === 'uploading' && (
        <span className="composer-file__state" role="status">
          <LoaderCircle className="spin" aria-hidden="true" />
          <span className="sr-only">Uploading {name}</span>
        </span>
      )}
      {item.error && (
        <button
          type="button"
          className="composer-file__state composer-file__retry"
          aria-label={`Upload ${name} again: ${item.error}`}
          onClick={() => attachments.retry(item)}
        >
          <RotateCw aria-hidden="true" />
        </button>
      )}
      <button
        type="button"
        className="composer-file__remove"
        aria-label={`Remove ${name}`}
        onClick={() => attachments.remove(item.key)}
      >
        <X aria-hidden="true" />
      </button>
    </li>
  )
}

const carriesFiles = (event: DragEvent) =>
  event.dataTransfer.types.includes('Files')

export function useDropZone(add: (files: File[]) => void) {
  const [dropping, setDropping] = useState(false)
  return {
    dropping,
    handlers: {
      onDragOver: (event: DragEvent) => {
        if (!carriesFiles(event)) return
        event.preventDefault()
        event.dataTransfer.dropEffect = 'copy'
        setDropping(true)
      },
      onDragLeave: (event: DragEvent) => {
        if (!event.currentTarget.contains(event.relatedTarget as Node | null))
          setDropping(false)
      },
      onDrop: (event: DragEvent) => {
        if (!carriesFiles(event)) return
        event.preventDefault()
        setDropping(false)
        add([...event.dataTransfer.files])
      },
    },
  }
}
