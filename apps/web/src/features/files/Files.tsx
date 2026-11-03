import { Download, FolderPlus, Share2, Upload } from 'lucide-react'
import { useEffect, useRef, useState } from 'react'
import { api, ApiError, failure, ok } from '../../api/client'
import type { components } from '../../api/schema'
import { Alert, Dialog, Empty, Status, when } from '../../components/primitives'
import './files.css'

export type FileEntry = components['schemas']['FileEntry']
type FileShare = components['schemas']['FileShare']
/** A folder opened from the top of a view, with what you may do in it. */
type Crumb = { id: string; name: string; access: FileEntry['access'] }
export type View = 'mine' | 'shared'

/** Shared files (ADRs 0059 and 0060): your folders, what others shared with you, uploads by
 *  drag and drop, downloads, and sharing with people, teams and agents. `only` narrows the
 *  shared view to one agent's files. */
export function Files({ only }: { only?: { agent: string } }) {
  const [view, setView] = useState<View>(only ? 'shared' : 'mine')
  const [path, setPath] = useState<Crumb[]>([])
  const [entries, setEntries] = useState<FileEntry[]>([])
  const [sharing, setSharing] = useState<FileEntry | null>(null)
  const [dragging, setDragging] = useState(false)
  const [status, setStatus] = useState('')
  const [error, setError] = useState('')
  const [loaded, setLoaded] = useState(false)
  const folder = path.at(-1)
  const writable = folder ? folder.access !== 'read' : view === 'mine'
  async function load() {
    setEntries(await list(view, folder?.id))
  }
  useEffect(() => {
    // A late answer for a folder the person already left must not replace the current one.
    let live = true
    void list(view, folder?.id)
      .then((items) => {
        if (!live) return
        setEntries(items)
        setError('')
      })
      .catch(
        (error) =>
          live &&
          setError(
            error instanceof ApiError && error.status === 503
              ? 'Shared file storage is not configured for this installation (CUBEBOT_STORAGE_DIRECTORY).'
              : failure(error),
          ),
      )
      .finally(() => live && setLoaded(true))
    return () => {
      live = false
    }
  }, [view, folder?.id])
  async function run(action: () => Promise<unknown>) {
    setError('')
    try {
      await action()
      await load()
    } catch (error) {
      setError(failure(error))
    }
  }
  async function upload(files: FileList | null) {
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
  const shown =
    only && !folder
      ? entries.filter((entry) => entry.owner_agent === only.agent)
      : entries
  return (
    <section
      className={dragging ? 'file-drop dragging' : 'file-drop'}
      onDragOver={(event) => {
        if (!writable || !event.dataTransfer.types.includes('Files')) return
        event.preventDefault()
        setDragging(true)
      }}
      onDragLeave={() => setDragging(false)}
      onDrop={(event) => {
        setDragging(false)
        if (!event.dataTransfer.files.length) return
        event.preventDefault()
        void upload(event.dataTransfer.files)
      }}
    >
      {!only && (
        <ViewTabs
          view={view}
          onPick={(kind) => {
            setView(kind)
            setPath([])
          }}
        />
      )}
      <Breadcrumbs
        root={rootLabel(only, view)}
        view={view}
        path={path}
        onOpen={setPath}
        onMove={(id, name, parent) => void move(id, parent, name)}
      />
      {writable && (
        <Toolbar
          onUpload={(files) => void upload(files)}
          onFolder={(name) =>
            run(() =>
              ok(
                api.POST('/api/files/folders', {
                  body: { parent: folder?.id ?? null, name },
                }),
              ),
            )
          }
        />
      )}
      <Status>{status}</Status>
      <Alert>{error}</Alert>
      {loaded && shown.length === 0 && (
        <Empty title={emptyTitle(view, folder)}>
          {writable ? 'Drop files here to upload them.' : ''}
        </Empty>
      )}
      <ul className="list files">
        {shown.map((entry) => (
          <Row
            key={entry.id}
            entry={entry}
            onOpen={() =>
              setPath([
                ...path,
                { id: entry.id, name: entry.name, access: entry.access },
              ])
            }
            onShare={() => setSharing(entry)}
            onRename={(name) => void move(entry.id, entry.parent ?? null, name)}
            onMoveInto={(id, name) => void move(id, entry.id, name)}
            onDelete={() =>
              run(() =>
                ok(
                  api.DELETE('/api/files/{file}', {
                    params: { path: { file: entry.id } },
                  }),
                ),
              )
            }
          />
        ))}
      </ul>
      {sharing && (
        <ShareDialog entry={sharing} onClose={() => setSharing(null)} />
      )}
    </section>
  )
}

function rootLabel(only: { agent: string } | undefined, view: View) {
  if (only) return 'Agent files'
  return view === 'mine' ? 'My files' : 'Shared with me'
}

function emptyTitle(view: View, folder: Crumb | undefined) {
  return view === 'shared' && !folder
    ? 'Nothing shared here yet.'
    : 'This folder is empty.'
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
          <span key={depth}>
            {depth > 0 && <span className="faint"> / </span>}
            <button
              type="button"
              className="link-btn"
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

function Toolbar({
  onUpload,
  onFolder,
}: {
  onUpload: (files: FileList | null) => void
  onFolder: (name: string) => Promise<void>
}) {
  const input = useRef<HTMLInputElement>(null)
  return (
    <form
      className="actions"
      onSubmit={(event) => {
        event.preventDefault()
        const form = event.currentTarget
        const name = String(new FormData(form).get('folder') ?? '').trim()
        if (name) void onFolder(name).then(() => form.reset())
      }}
    >
      <button
        type="button"
        className="btn btn--primary btn--sm"
        onClick={() => input.current?.click()}
      >
        <Upload /> Upload files
      </button>
      <input
        ref={input}
        type="file"
        multiple
        hidden
        onChange={(event) => {
          onUpload(event.currentTarget.files)
          event.currentTarget.value = ''
        }}
      />
      <input
        className="input"
        name="folder"
        maxLength={255}
        placeholder="New folder name"
        style={{ width: 180 }}
      />
      <button className="btn btn--sm">
        <FolderPlus /> Create folder
      </button>
    </form>
  )
}

function Row({
  entry,
  onOpen,
  onShare,
  onRename,
  onMoveInto,
  onDelete,
}: {
  entry: FileEntry
  onOpen: () => void
  onShare: () => void
  onRename: (name: string) => void
  onMoveInto: (id: string, name: string) => void
  onDelete: () => void
}) {
  const [renaming, setRenaming] = useState(false)
  const writable = entry.access !== 'read'
  return (
    <li
      className="files__row"
      draggable={writable}
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
      {renaming ? (
        <form
          className="actions"
          onSubmit={(event) => {
            event.preventDefault()
            const name = String(new FormData(event.currentTarget).get('name'))
            setRenaming(false)
            if (name.trim() && name !== entry.name) onRename(name.trim())
          }}
        >
          <input
            className="input"
            name="name"
            aria-label="New name"
            required
            maxLength={255}
            defaultValue={entry.name}
            autoFocus
            style={{ width: 240 }}
          />
          <button className="btn btn--sm">Rename</button>
          <button
            type="button"
            className="btn btn--ghost btn--sm"
            onClick={() => setRenaming(false)}
          >
            Cancel
          </button>
        </form>
      ) : (
        <div className="title">
          {entry.folder ? (
            <button type="button" className="link-btn" onClick={onOpen}>
              {entry.name}/
            </button>
          ) : (
            entry.name
          )}
          {entry.owner_agent && <span className="chip">from an agent</span>}
          {entry.access === 'read' && <span className="chip">read only</span>}
        </div>
      )}
      <div className="actions files__meta">
        <span className="faint small">
          {entry.folder ? 'Folder' : size(entry.size ?? 0)} ·{' '}
          {when(entry.updated_at)}
        </span>
        {!entry.folder && (
          <a
            href={`/api/files/${entry.id}/content`}
            download={entry.name}
            className="small"
          >
            <Download size={12} style={{ verticalAlign: '-2px' }} /> Download
          </a>
        )}
        {entry.access === 'manage' && (
          <button type="button" className="link-btn small" onClick={onShare}>
            <Share2 size={12} style={{ verticalAlign: '-2px' }} /> Share
          </button>
        )}
        {writable && (
          <>
            <button
              type="button"
              className="link-btn small"
              onClick={() => setRenaming(true)}
            >
              Rename
            </button>
            <button
              type="button"
              className="link-btn small"
              onClick={() => {
                const what = entry.folder
                  ? `${entry.name} and everything in it`
                  : entry.name
                if (window.confirm(`Delete ${what}? This cannot be undone.`))
                  onDelete()
              }}
            >
              Delete
            </button>
          </>
        )}
      </div>
    </li>
  )
}

function size(bytes: number) {
  if (bytes < 1024) return `${bytes} B`
  const units = ['KiB', 'MiB', 'GiB', 'TiB']
  let value = bytes / 1024
  let unit = 0
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024
    unit += 1
  }
  return `${value.toFixed(1)} ${units[unit]}`
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
