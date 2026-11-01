import { useEffect, useRef, useState } from 'react'
import { api, ApiError, failure, ok } from '../api/client'
import type { components } from '../api/schema'

type FileEntry = components['schemas']['FileEntry']
type FileShare = components['schemas']['FileShare']
/** A folder opened from the top of a view, with what you may do in it. */
type Crumb = { id: string; name: string; access: FileEntry['access'] }
type View = 'mine' | 'shared'

/** Shared files (ADRs 0059 and 0060): your folders, what others shared with you, uploads by
 *  drag and drop, downloads, and sharing with people, teams and agents. */
export function Files({ onClose }: { onClose: () => void }) {
  const [view, setView] = useState<View>('mine')
  const [path, setPath] = useState<Crumb[]>([])
  const [entries, setEntries] = useState<FileEntry[]>([])
  const [sharing, setSharing] = useState<FileEntry | null>(null)
  const [dragging, setDragging] = useState(false)
  const [status, setStatus] = useState('')
  const [error, setError] = useState('')
  const folder = path.at(-1)
  const writable = folder ? folder.access !== 'read' : view === 'mine'
  async function load() {
    setEntries(await list(view, folder?.id))
  }
  useEffect(() => {
    void list(view, folder?.id)
      .then(setEntries)
      .catch((error) => setError(failure(error)))
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
  function open(entry: FileEntry) {
    setPath([...path, { id: entry.id, name: entry.name, access: entry.access }])
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
      <header className="conversation-header">
        <div>
          <h1>Files</h1>
          <p className="muted">
            Shared files stay available while agents sleep. Drop files here to
            upload them.
          </p>
        </div>
        <div className="actions">
          <button className="secondary" onClick={onClose}>
            Close
          </button>
        </div>
      </header>
      <nav className="actions" aria-label="File views">
        {(['mine', 'shared'] as const).map((kind) => (
          <button
            key={kind}
            className={view === kind ? undefined : 'secondary'}
            aria-pressed={view === kind}
            onClick={() => {
              setView(kind)
              setPath([])
            }}
          >
            {kind === 'mine' ? 'My files' : 'Shared with me'}
          </button>
        ))}
      </nav>
      <Breadcrumbs
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
      {status && <p role="status">{status}</p>}
      {error && <p role="alert">{error}</p>}
      {entries.length === 0 && (
        <p className="muted">
          {view === 'shared' && !folder
            ? 'Nothing is shared with you yet.'
            : 'This folder is empty.'}
        </p>
      )}
      <ul className="memory-list">
        {entries.map((entry) => (
          <Row
            key={entry.id}
            entry={entry}
            onOpen={() => open(entry)}
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

function list(view: View, parent: string | undefined) {
  if (parent)
    return ok(api.GET('/api/files', { params: { query: { parent } } }))
  if (view === 'shared') return ok(api.GET('/api/files/shared'))
  return ok(api.GET('/api/files', { params: { query: {} } }))
}

/** Streams the file as the request body; the request id lets a retry find the first attempt's
 *  file. */
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
  view,
  path,
  onOpen,
  onMove,
}: {
  view: View
  path: Crumb[]
  onOpen: (path: Crumb[]) => void
  onMove: (id: string, name: string, parent: string | null) => void
}) {
  const crumbs = [
    { label: view === 'mine' ? 'My files' : 'Shared with me', depth: 0 },
    ...path.map((crumb, index) => ({ label: crumb.name, depth: index + 1 })),
  ]
  return (
    <nav className="memory-meta" aria-label="Folder">
      {crumbs.map(({ label, depth }) => {
        // Moving to the top level is only into your own.
        const above = path[depth - 1]
        const target = above ? above.id : view === 'mine' ? null : undefined
        return (
          <span key={depth}>
            {depth > 0 && '/ '}
            <button
              className="text-button"
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
      <button type="button" onClick={() => input.current?.click()}>
        Upload files
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
      <label>
        <span className="muted">New folder</span>{' '}
        <input name="folder" maxLength={255} placeholder="Folder name" />
      </label>
      <button className="secondary">Create folder</button>
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
      className="memory"
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
            name="name"
            aria-label="New name"
            required
            maxLength={255}
            defaultValue={entry.name}
            autoFocus
          />
          <button>Rename</button>
          <button
            type="button"
            className="secondary"
            onClick={() => setRenaming(false)}
          >
            Cancel
          </button>
        </form>
      ) : (
        <p>
          {entry.folder ? (
            <button className="text-button" onClick={onOpen}>
              <strong>{entry.name}/</strong>
            </button>
          ) : (
            <strong>{entry.name}</strong>
          )}
        </p>
      )}
      <div className="memory-meta">
        <span className="muted">
          {entry.folder ? 'Folder' : size(entry.size ?? 0)}
          {entry.owner_agent && ', from an agent'}
          {entry.access === 'read' && ', read only'},{' '}
          {new Date(entry.updated_at).toLocaleString()}
        </span>
        {!entry.folder && (
          <a href={`/api/files/${entry.id}/content`} download={entry.name}>
            Download
          </a>
        )}
        {entry.access === 'manage' && (
          <button className="text-button" onClick={onShare}>
            Share
          </button>
        )}
        {writable && (
          <>
            <button className="text-button" onClick={() => setRenaming(true)}>
              Rename
            </button>
            <button
              className="text-button"
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
  const dialog = useRef<HTMLDialogElement>(null)
  const [shares, setShares] = useState<FileShare[] | null>(null)
  const [error, setError] = useState('')
  const [busy, setBusy] = useState(false)
  useEffect(() => {
    dialog.current?.showModal()
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
    <dialog ref={dialog} onClose={onClose} aria-labelledby="share-title">
      <form
        method="dialog"
        onSubmit={(event) => {
          event.preventDefault()
          void save()
        }}
      >
        <h2 id="share-title">Share {entry.name}</h2>
        {entry.folder && (
          <p className="muted">Shares reach everything inside this folder.</p>
        )}
        {!shares && !error && <p role="status">Loading shares…</p>}
        {shares?.length === 0 && <p className="muted">Not shared yet.</p>}
        {shares?.map((share, index) => (
          <fieldset key={index} className="actions">
            <legend className="muted">Share {index + 1}</legend>
            <select
              aria-label="Shared with"
              value={share.kind}
              onChange={(event) =>
                change(index, {
                  kind: event.target.value as FileShare['kind'],
                })
              }
            >
              <option value="user">Person</option>
              <option value="team">Team</option>
              <option value="agent">Agent</option>
            </select>
            <input
              aria-label={share.kind === 'user' ? 'Login' : 'Name'}
              required
              maxLength={255}
              placeholder={share.kind === 'user' ? 'Login' : 'Exact name'}
              value={share.name}
              onChange={(event) => change(index, { name: event.target.value })}
            />
            <select
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
              className="text-button"
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
        {error && <p role="alert">{error}</p>}
        <div className="actions">
          <button
            type="button"
            className="secondary"
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
          <button disabled={busy || !shares}>
            {busy ? 'Saving…' : 'Save shares'}
          </button>
          <button type="button" className="secondary" onClick={onClose}>
            Cancel
          </button>
        </div>
      </form>
    </dialog>
  )
}
