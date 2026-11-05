import {
  ChevronRight,
  Download,
  File as FileIcon,
  FileImage,
  FileText,
  Folder,
  FolderPlus,
  MoreHorizontal,
  Pencil,
  Share2,
  Trash2,
  Upload,
} from 'lucide-react'
import { useNavigate, useRouter, useSearch } from '@tanstack/react-router'
import { useEffect, useRef, useState } from 'react'
import { api, ApiError, failure, ok } from '../../api/client'
import type { components } from '../../api/schema'
import {
  Alert,
  Dialog,
  Menu,
  Skeleton,
  Status,
  when,
} from '../../components/primitives'
import { FileViewer } from './FileViewer'
import { fileSize } from './viewer'
import './files.css'

export type FileEntry = components['schemas']['FileEntry']
type FileShare = components['schemas']['FileShare']
/** A folder opened from the top of a view, with what you may do in it. */
type Crumb = { id: string; name: string; access: FileEntry['access'] }
export type View = 'mine' | 'shared'

/** Shared files (docs/adr/memory.md#orchestrator-managed-file-storage-on-a-persistent-volume and docs/adr/memory.md#native-cubebot-storage-api): your folders, what others shared with you, uploads by
 *  drag and drop, downloads, and sharing with people, teams and agents. `only` narrows the
 *  shared view to one agent's files. */
export function Files({ only }: { only?: { agent: string } }) {
  const [view, setView] = useState<View>(only ? 'shared' : 'mine')
  const [path, setPath] = useState<Crumb[]>([])
  const [sharing, setSharing] = useState<FileEntry | null>(null)
  const [creating, setCreating] = useState(false)
  const [status, setStatus] = useState('')
  const upload = useRef<HTMLInputElement>(null)
  const folder = path.at(-1)
  const writable = folder ? folder.access !== 'read' : view === 'mine'
  const { entries, loaded, error, setError, load } = useFolder(view, folder?.id)
  async function run(action: () => Promise<unknown>) {
    setError('')
    try {
      await action()
      await load()
    } catch (error) {
      setError(failure(error))
    }
  }
  async function send(files: FileList | null) {
    if (!files?.length || !writable) return
    await run(async () => {
      for (const [index, file] of [...files].entries()) {
        setStatus(`Uploading ${file.name} (${index + 1} of ${files.length})…`)
        await uploadFile(file, folder?.id)
      }
    }).finally(() => setStatus(''))
  }
  const move = (id: string, parent: string | null, name: string) =>
    run(() =>
      ok(
        api.PATCH('/api/files/{file}', {
          params: { path: { file: id } },
          body: { parent, name },
        }),
      ),
    )
  const actions: Actions = {
    onUpload: () => upload.current?.click(),
    onFolder: () => setCreating(true),
  }
  const shown = visible(entries, only, folder)
  const drop = useFileDrop(writable, (files) => void send(files))
  const viewer = useOpenFile()
  return (
    <section
      className={drop.dragging ? 'file-drop dragging' : 'file-drop'}
      data-folder={folder?.name ?? rootLabel(only, view)}
      {...drop.handlers}
    >
      <input
        ref={upload}
        type="file"
        multiple
        hidden
        onChange={(event) => {
          void send(event.currentTarget.files)
          event.currentTarget.value = ''
        }}
      />
      <FilesBar
        root={rootLabel(only, view)}
        view={only ? null : view}
        path={path}
        // An empty folder offers them itself, so the surface keeps one primary action.
        actions={writable && (creating || shown.length > 0) ? actions : null}
        onView={(kind) => {
          setView(kind)
          setPath([])
          setCreating(false)
        }}
        onOpen={(next) => {
          setPath(next)
          setCreating(false)
        }}
        onMove={(id, name, parent) => void move(id, parent, name)}
      />
      <Status>{status}</Status>
      <Alert>{error}</Alert>
      {!loaded && !error && <Skeleton lines={3} />}
      {loaded && (
        <Listing
          entries={shown}
          creating={creating}
          empty={
            <EmptyFolder
              view={view}
              inFolder={Boolean(folder)}
              writable={writable}
              {...actions}
            />
          }
          onCancelFolder={() => setCreating(false)}
          onCreateFolder={(name) =>
            run(async () => {
              await ok(
                api.POST('/api/files/folders', {
                  body: { parent: folder?.id ?? null, name },
                }),
              )
              setCreating(false)
            })
          }
          row={(entry) => ({
            onOpen: () => {
              if (!entry.folder) return viewer.show(entry.id)
              setCreating(false)
              setPath([
                ...path,
                { id: entry.id, name: entry.name, access: entry.access },
              ])
            },
            onShare: () => setSharing(entry),
            onRename: (name) => void move(entry.id, entry.parent ?? null, name),
            onMoveInto: (id, name) => void move(id, entry.id, name),
            onDelete: () =>
              run(() =>
                ok(
                  api.DELETE('/api/files/{file}', {
                    params: { path: { file: entry.id } },
                  }),
                ),
              ),
          })}
        />
      )}
      {sharing && (
        <ShareDialog entry={sharing} onClose={() => setSharing(null)} />
      )}
      <OpenFile entries={entries} open={viewer.open} onClose={viewer.close} />
    </section>
  )
}

/** A folder's entries, reloaded when the view or folder changes; a late answer for a folder
 *  the person already left never replaces the current one. */
function OpenFile({
  entries,
  open,
  onClose,
}: {
  entries: FileEntry[]
  open: string | undefined
  onClose: () => void
}) {
  const entry = entries.find((item) => item.id === open && !item.folder)
  if (!entry) return null
  return (
    <FileViewer
      file={{
        name: entry.name,
        content_type: entry.content_type,
        size: entry.size ?? 0,
        url: `/api/files/${entry.id}/content`,
      }}
      onClose={onClose}
    />
  )
}

/** The open file lives in the URL, so a phone's back gesture closes it and a link reopens it. */
function useOpenFile() {
  const search = useSearch({ strict: false }) as { open?: string }
  const navigate = useNavigate()
  const router = useRouter()
  const pushed = useRef(false)
  const to = (open: string | undefined, replace: boolean) =>
    void navigate({
      to: '.',
      replace,
      search: ((previous: Record<string, unknown>) => ({
        ...previous,
        open,
      })) as never,
    })
  return {
    open: search.open,
    show: (id: string) => {
      pushed.current = true
      to(id, false)
    },
    close: () => {
      if (pushed.current) router.history.back()
      else to(undefined, true)
      pushed.current = false
    },
  }
}

function useFolder(view: View, folder: string | undefined) {
  const [entries, setEntries] = useState<FileEntry[]>([])
  const [error, setError] = useState('')
  const [loaded, setLoaded] = useState(false)
  // Only the newest listing shows, so a late one cannot put folder A's files in folder B.
  const latest = useRef(0)
  useEffect(() => {
    let live = true
    latest.current++
    void list(view, folder)
      .then((items) => {
        if (!live) return
        setEntries(items)
        setError('')
      })
      .catch((error) => live && setError(listProblem(error)))
      .finally(() => live && setLoaded(true))
    return () => {
      live = false
    }
  }, [view, folder])
  const load = async () => {
    const ticket = ++latest.current
    const items = await list(view, folder)
    if (ticket === latest.current) setEntries(items)
  }
  return { entries, loaded, error, setError, load }
}

function listProblem(error: unknown) {
  return error instanceof ApiError && error.status === 503
    ? 'Shared file storage is not configured for this installation (CUBEBOT_STORAGE_DIRECTORY).'
    : failure(error)
}

type Actions = { onUpload: () => void; onFolder: () => void }
type RowActions = {
  onOpen: () => void
  onShare: () => void
  onRename: (name: string) => void
  onMoveInto: (id: string, name: string) => void
  onDelete: () => void
}

/** The folder's rows, a new folder's name row while one is being made, or `empty`. */
function Listing({
  entries,
  creating,
  empty,
  onCreateFolder,
  onCancelFolder,
  row,
}: {
  entries: FileEntry[]
  creating: boolean
  empty: React.ReactNode
  onCreateFolder: (name: string) => Promise<void>
  onCancelFolder: () => void
  row: (entry: FileEntry) => RowActions
}) {
  if (!creating && entries.length === 0) return empty
  return (
    <ul className="entries files" aria-label="Files">
      {creating && (
        <NewFolderRow onCreate={onCreateFolder} onCancel={onCancelFolder} />
      )}
      {entries.map((entry) => (
        <Row key={entry.id} entry={entry} {...row(entry)} />
      ))}
    </ul>
  )
}

/** Files dragged in from the desktop, while the folder takes them. */
function useFileDrop(writable: boolean, onFiles: (files: FileList) => void) {
  const [dragging, setDragging] = useState(false)
  const handlers = {
    onDragOver: (event: React.DragEvent) => {
      if (!writable || !event.dataTransfer.types.includes('Files')) return
      event.preventDefault()
      setDragging(true)
    },
    onDragLeave: (event: React.DragEvent) => {
      if (!event.currentTarget.contains(event.relatedTarget as Node | null))
        setDragging(false)
    },
    onDrop: (event: React.DragEvent) => {
      setDragging(false)
      if (!event.dataTransfer.files.length) return
      event.preventDefault()
      onFiles(event.dataTransfer.files)
    },
  }
  return { dragging, handlers }
}

/** The view tabs and the folder's actions, then the path once inside a folder. */
function FilesBar({
  root,
  view,
  path,
  actions,
  onView,
  onOpen,
  onMove,
}: {
  root: string
  /** `null` in an agent's panel, which has one view. */
  view: View | null
  path: Crumb[]
  actions: Actions | null
  onView: (view: View) => void
  onOpen: (path: Crumb[]) => void
  onMove: (id: string, name: string, parent: string | null) => void
}) {
  return (
    <>
      <div className="files-bar">
        {view && <ViewTabs view={view} onPick={onView} />}
        <span className="files-bar__space" />
        {actions && <Toolbar {...actions} />}
      </div>
      {path.length > 0 && (
        <Breadcrumbs
          root={root}
          view={view ?? 'shared'}
          path={path}
          onOpen={onOpen}
          onMove={onMove}
        />
      )}
    </>
  )
}

/** At the top of an agent's panel, only that agent's files; inside a folder, all of it. */
function visible(
  entries: FileEntry[],
  only: { agent: string } | undefined,
  folder: Crumb | undefined,
) {
  if (!only || folder) return entries
  return entries.filter((entry) => entry.owner_agent === only.agent)
}

function rootLabel(only: { agent: string } | undefined, view: View) {
  if (only) return 'Agent files'
  return view === 'mine' ? 'My files' : 'Shared with me'
}

function ViewTabs({
  view,
  onPick,
}: {
  view: View
  onPick: (view: View) => void
}) {
  return (
    <nav className="tabs" aria-label="File views">
      {(['mine', 'shared'] as const).map((kind) => (
        <button
          key={kind}
          type="button"
          aria-selected={view === kind}
          onClick={() => onPick(kind)}
        >
          {kind === 'mine' ? 'My files' : 'Shared with me'}
        </button>
      ))}
    </nav>
  )
}

function Toolbar({ onUpload, onFolder }: Actions) {
  return (
    <div className="actions">
      <button type="button" className="btn btn--sm" onClick={onFolder}>
        <FolderPlus /> New folder
      </button>
      <button
        type="button"
        className="btn btn--primary btn--sm"
        onClick={onUpload}
      >
        <Upload /> Upload
      </button>
    </div>
  )
}

/** An empty folder teaches the drop: the whole pane takes files. */
function EmptyFolder({
  view,
  inFolder,
  writable,
  onUpload,
  onFolder,
}: Actions & { view: View; inFolder: boolean; writable: boolean }) {
  if (!writable)
    return (
      <div className="files-empty">
        <Folder className="files-empty__icon" />
        <h3>
          {view === 'shared' && !inFolder
            ? 'Nothing is shared with you yet.'
            : 'This folder is empty.'}
        </h3>
        <p>
          {view === 'shared' && !inFolder
            ? 'When a person or an agent shares a file or folder with you, it appears here.'
            : 'You can view this folder; only people who can edit it add files.'}
        </p>
      </div>
    )
  return (
    <div className="files-empty">
      <Upload className="files-empty__icon" />
      <h3>{inFolder ? 'This folder is empty.' : 'No files yet.'}</h3>
      <p>
        Drop files anywhere on this page to upload them. Agents read files you
        share with them, even while they sleep.
      </p>
      <div className="actions">
        <button type="button" className="btn btn--primary" onClick={onUpload}>
          <Upload /> Upload files
        </button>
        <button type="button" className="btn" onClick={onFolder}>
          <FolderPlus /> New folder
        </button>
      </div>
    </div>
  )
}

function NewFolderRow({
  onCreate,
  onCancel,
}: {
  onCreate: (name: string) => Promise<void>
  onCancel: () => void
}) {
  return (
    <li className="entry">
      <span className="entry__icon" aria-hidden="true">
        <FolderPlus />
      </span>
      <form
        className="files-name"
        onSubmit={(event) => {
          event.preventDefault()
          const name = String(new FormData(event.currentTarget).get('name'))
          if (name.trim()) void onCreate(name.trim())
        }}
      >
        <input
          className="input"
          name="name"
          aria-label="Folder name"
          placeholder="Folder name"
          required
          maxLength={255}
          autoFocus
          onKeyDown={(event) => event.key === 'Escape' && onCancel()}
        />
        <button className="btn btn--primary btn--sm">Create</button>
        <button
          type="button"
          className="btn btn--ghost btn--sm"
          onClick={onCancel}
        >
          Cancel
        </button>
      </form>
      <span />
    </li>
  )
}

function list(view: View, parent: string | undefined) {
  if (parent)
    return ok(api.GET('/api/files', { params: { query: { parent } } }))
  if (view === 'shared') return ok(api.GET('/api/files/shared'))
  return ok(api.GET('/api/files', { params: { query: {} } }))
}

/** Streams the file as the request body. */
async function uploadFile(file: File, parent: string | undefined) {
  const query = new URLSearchParams({
    name: file.name,
    request_id: crypto.randomUUID(),
  })
  if (parent) query.set('parent', parent)
  const response = await fetch(`/api/files?${query}`, {
    method: 'POST',
    body: file,
    credentials: 'same-origin',
    headers: { 'content-type': file.type || 'application/octet-stream' },
  })
  if (response.status === 507)
    throw new Error(
      'The installation is out of file storage. Delete files or ask an administrator.',
    )
  if (response.status === 409)
    throw new Error(`A file named ${file.name} is already in this folder.`)
  if (!response.ok) throw new ApiError(response.status)
}

/** Each folder above the current one; dropping a row on one moves it there. */
function Breadcrumbs({
  root,
  view,
  path,
  onOpen,
  onMove,
}: {
  root: string
  view: View
  path: Crumb[]
  onOpen: (path: Crumb[]) => void
  onMove: (id: string, name: string, parent: string | null) => void
}) {
  const crumbs = [
    { label: root, depth: 0 },
    ...path.map((crumb, index) => ({ label: crumb.name, depth: index + 1 })),
  ]
  return (
    <nav className="crumbs" aria-label="Folder">
      {crumbs.map(({ label, depth }) => {
        // Moving to the top level is only into your own.
        const above = path[depth - 1]
        const target = above ? above.id : view === 'mine' ? null : undefined
        return (
          <span key={depth} className="crumbs__step">
            {depth > 0 && (
              <ChevronRight className="crumbs__sep" aria-hidden="true" />
            )}
            <button
              type="button"
              className="crumbs__link"
              aria-current={depth === path.length ? 'page' : undefined}
              onClick={() => onOpen(path.slice(0, depth))}
              onDragOver={(event) => {
                if (target !== undefined && dragged(event))
                  event.preventDefault()
              }}
              onDrop={(event) => {
                const row = dragged(event)
                if (target === undefined || !row) return
                event.preventDefault()
                event.stopPropagation()
                onMove(row.id, row.name, target)
              }}
            >
              {label}
            </button>
          </span>
        )
      })}
    </nav>
  )
}

const ROW = 'application/x-cubebot-file'
function dragged(event: React.DragEvent): { id: string; name: string } | null {
  if (!event.dataTransfer.types.includes(ROW)) return null
  const data = event.dataTransfer.getData(ROW)
  // Browsers hide the data until the drop; the type alone admits the drag.
  return data ? JSON.parse(data) : { id: '', name: '' }
}

const IMAGE = /\.(png|jpe?g|gif|webp|svg|avif)$/i
const TEXT = /\.(md|txt|csv|json|ya?ml|log|pdf|docx?)$/i
function EntryIcon({ entry }: { entry: FileEntry }) {
  if (entry.folder) return <Folder />
  if (IMAGE.test(entry.name)) return <FileImage />
  if (TEXT.test(entry.name)) return <FileText />
  return <FileIcon />
}

function Row({
  entry,
  onOpen,
  onShare,
  onRename,
  onMoveInto,
  onDelete,
}: RowActions & { entry: FileEntry }) {
  const [renaming, setRenaming] = useState(false)
  const writable = entry.access !== 'read'
  return (
    <li
      className={
        entry.folder
          ? 'entry files__row files__row--folder'
          : 'entry files__row'
      }
      draggable={writable && !renaming}
      onDragStart={(event) =>
        event.dataTransfer.setData(
          ROW,
          JSON.stringify({ id: entry.id, name: entry.name }),
        )
      }
      onDragOver={(event) => {
        if (entry.folder && writable && dragged(event)) event.preventDefault()
      }}
      onDrop={(event) => {
        const row = dragged(event)
        if (!entry.folder || !row || row.id === entry.id) return
        event.preventDefault()
        event.stopPropagation()
        onMoveInto(row.id, row.name)
      }}
    >
      <span className="entry__icon" aria-hidden="true">
        <EntryIcon entry={entry} />
      </span>
      {renaming ? (
        <RenameForm
          name={entry.name}
          onDone={(name) => {
            setRenaming(false)
            if (name && name !== entry.name) onRename(name)
          }}
        />
      ) : (
        <div className="entry__main">
          <button
            type="button"
            className="files__open entry__name"
            onClick={onOpen}
          >
            {entry.name}
          </button>
          <span className="entry__detail">
            {entry.owner_agent && 'From an agent · '}
            {entry.access === 'read' && 'View only · '}
            {entry.folder ? 'Folder' : fileSize(entry.size ?? 0)}
          </span>
        </div>
      )}
      <div className="entry__meta">
        <span className="files__when">{when(entry.updated_at)}</span>
        <RowMenu
          entry={entry}
          onShare={onShare}
          onRename={() => setRenaming(true)}
          onDelete={() => {
            const what = entry.folder
              ? `${entry.name} and everything in it`
              : entry.name
            if (window.confirm(`Delete ${what}? This cannot be undone.`))
              onDelete()
          }}
        />
      </div>
    </li>
  )
}

function RenameForm({
  name,
  onDone,
}: {
  name: string
  onDone: (name: string) => void
}) {
  return (
    <form
      className="files-name"
      onSubmit={(event) => {
        event.preventDefault()
        onDone(String(new FormData(event.currentTarget).get('name')).trim())
      }}
    >
      <input
        className="input"
        name="name"
        aria-label="New name"
        required
        maxLength={255}
        defaultValue={name}
        autoFocus
        onFocus={(event) => {
          // Selects the name without its extension, as file managers do.
          const dot = name.lastIndexOf('.')
          event.currentTarget.setSelectionRange(0, dot > 0 ? dot : name.length)
        }}
        onKeyDown={(event) => event.key === 'Escape' && onDone(name)}
      />
      <button className="btn btn--primary btn--sm">Rename</button>
      <button
        type="button"
        className="btn btn--ghost btn--sm"
        onClick={() => onDone(name)}
      >
        Cancel
      </button>
    </form>
  )
}

function RowMenu({
  entry,
  onShare,
  onRename,
  onDelete,
}: {
  entry: FileEntry
  onShare: () => void
  onRename: () => void
  onDelete: () => void
}) {
  const writable = entry.access !== 'read'
  // A folder you only view has nothing to download, share or change.
  if (entry.folder && !writable) return <span />
  return (
    <Menu
      className="menu--right"
      trigger={<MoreHorizontal />}
      triggerClassName="btn btn--ghost btn--icon btn--sm"
      label={`Actions for ${entry.name}`}
    >
      {!entry.folder && (
        <a
          href={`/api/files/${entry.id}/content`}
          download={entry.name}
          className="menu__item"
        >
          <Download /> Download
        </a>
      )}
      {entry.access === 'manage' && (
        <button type="button" className="menu__item" onClick={onShare}>
          <Share2 /> Share
        </button>
      )}
      {writable && (
        <button type="button" className="menu__item" onClick={onRename}>
          <Pencil /> Rename
        </button>
      )}
      {writable && (
        <button
          type="button"
          className="menu__item menu__item--danger"
          onClick={onDelete}
        >
          <Trash2 /> Delete
        </button>
      )}
    </Menu>
  )
}

/** The entry's own shares; a folder's reach everything inside it. */
function ShareDialog({
  entry,
  onClose,
}: {
  entry: FileEntry
  onClose: () => void
}) {
  const [shares, setShares] = useState<FileShare[] | null>(null)
  const [error, setError] = useState('')
  const [busy, setBusy] = useState(false)
  useEffect(() => {
    void ok(
      api.GET('/api/files/{file}/shares', {
        params: { path: { file: entry.id } },
      }),
    )
      .then(setShares)
      .catch((error) => setError(failure(error)))
  }, [entry.id])
  const change = (index: number, share: Partial<FileShare>) =>
    setShares((current) =>
      (current ?? []).map((known, at) =>
        at === index ? { ...known, ...share } : known,
      ),
    )
  async function save() {
    setBusy(true)
    setError('')
    try {
      const body = (shares ?? []).filter((share) => share.name.trim())
      await ok(
        api.PUT('/api/files/{file}/shares', {
          params: { path: { file: entry.id } },
          body,
        }),
      )
      onClose()
    } catch (error) {
      setError(
        error instanceof ApiError && error.status === 400
          ? 'Each share names one active person by login, a team, or an agent by its exact name.'
          : failure(error),
      )
    } finally {
      setBusy(false)
    }
  }
  return (
    <Dialog open title={`Share ${entry.name}`} onClose={onClose}>
      <form
        className="form"
        onSubmit={(event) => {
          event.preventDefault()
          void save()
        }}
      >
        {entry.folder && (
          <p className="muted small">
            Shares reach everything inside this folder.
          </p>
        )}
        {!shares && !error && <p role="status">Loading shares…</p>}
        {shares?.length === 0 && <p className="muted small">Not shared yet.</p>}
        {shares?.map((share, index) => (
          <fieldset key={index} className="actions share-row">
            <legend className="sr-only">Share {index + 1}</legend>
            <select
              className="input"
              aria-label="Shared with"
              value={share.kind}
              onChange={(event) =>
                change(index, { kind: event.target.value as FileShare['kind'] })
              }
            >
              <option value="user">Person</option>
              <option value="team">Team</option>
              <option value="agent">Agent</option>
            </select>
            <input
              className="input"
              aria-label={share.kind === 'user' ? 'Login' : 'Name'}
              required
              maxLength={255}
              placeholder={share.kind === 'user' ? 'Login' : 'Exact name'}
              value={share.name}
              onChange={(event) => change(index, { name: event.target.value })}
            />
            <select
              className="input"
              aria-label="Permission"
              value={share.permission}
              onChange={(event) =>
                change(index, {
                  permission: event.target.value as FileShare['permission'],
                })
              }
            >
              <option value="read">Can view</option>
              <option value="write">Can edit</option>
            </select>
            <button
              type="button"
              className="link-btn small"
              onClick={() =>
                setShares((current) =>
                  (current ?? []).filter((_, at) => at !== index),
                )
              }
            >
              Remove
            </button>
          </fieldset>
        ))}
        <Alert>{error}</Alert>
        <div className="actions">
          <button
            type="button"
            className="btn btn--sm"
            disabled={!shares}
            onClick={() =>
              setShares((current) => [
                ...(current ?? []),
                { kind: 'user', name: '', permission: 'read' },
              ])
            }
          >
            Add person, team or agent
          </button>
          <button
            className="btn btn--primary btn--sm"
            disabled={busy || !shares}
          >
            {busy ? 'Saving…' : 'Save shares'}
          </button>
        </div>
      </form>
    </Dialog>
  )
}
