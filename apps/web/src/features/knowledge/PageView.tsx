import { useQuery, useQueryClient } from '@tanstack/react-query'
import { Link, useNavigate } from '@tanstack/react-router'
import {
  ChevronLeft,
  ChevronRight,
  File as FileIcon,
  FolderPlus,
  History,
  Paperclip,
  Plus,
  Trash2,
  X,
} from 'lucide-react'
import { lazy, Suspense, useCallback, useEffect, useRef, useState } from 'react'
import { api, failure, ok } from '../../api/client'
import { agentHover } from '../../components/agentHover'
import { Cube } from '../../components/Cube'
import {
  Alert,
  Dialog,
  Empty,
  Skeleton,
  when,
} from '../../components/primitives'
import { useAgents } from '../../app/queries'
import { useIsPhone } from '../shell/viewport'
import {
  attachmentUrl,
  kbKeys,
  savePage,
  trashPage,
  uploadAttachment,
  usePage,
  useRefreshTree,
  useTree,
  type Attachment,
  type Page,
} from './api'
import type { EditorHandle } from './Editor'
import { PageIcon, useCreate } from './Knowledge'
import { childrenOf, pathTo, titled } from './tree'
import { useSave, type SaveState } from './useSave'

const Editor = lazy(() =>
  import('./Editor').then((module) => ({ default: module.Editor })),
)

export function PageView({ id }: { id: string }) {
  const page = usePage(id)
  if (page.isPending) return <Skeleton lines={6} width="min(720px, 100%)" />
  if (!page.data)
    return (
      <Empty title="This page is gone.">
        It may have been moved to the trash. Look there, or pick another page.
      </Empty>
    )
  // Remounts per page, so typing state never carries over.
  return page.data.kind === 'folder' ? (
    <FolderView key={id} page={page.data} />
  ) : (
    <Document key={id} page={page.data} />
  )
}

function Bar({
  page,
  state,
  onHistory,
}: {
  page: Page
  state?: SaveState | undefined
  onHistory?: () => void
}) {
  const tree = useTree()
  const phone = useIsPhone()
  const navigate = useNavigate()
  const refresh = useRefreshTree()
  const path = pathTo(tree.data ?? [], page.id)
  return (
    <header className="kb-bar">
      {phone && (
        <Link
          to="/kb"
          className="btn btn--ghost btn--icon btn--sm"
          aria-label="All pages"
        >
          <ChevronLeft />
        </Link>
      )}
      <nav className="kb-bar__path" aria-label="Path">
        {path.map((step, index) => (
          <span key={step.id} className="kb-bar__step">
            {index > 0 && <ChevronRight aria-hidden="true" />}
            <Link to="/kb/$pageId" params={{ pageId: step.id }}>
              {titled(step)}
            </Link>
          </span>
        ))}
      </nav>
      {state && <SaveMark state={state} />}
      {onHistory && (
        <button
          type="button"
          className="btn btn--ghost btn--icon btn--sm"
          aria-label="Version history"
          title="Version history"
          onClick={onHistory}
        >
          <History />
        </button>
      )}
      <button
        type="button"
        className="btn btn--ghost btn--icon btn--sm"
        aria-label="Move to trash"
        title="Move to trash"
        onClick={async () => {
          await trashPage(page.id)
          await refresh()
          await navigate({ to: '/kb' })
        }}
      >
        <Trash2 />
      </button>
    </header>
  )
}

function SaveMark({ state }: { state: SaveState }) {
  const text = {
    saved: 'Saved',
    waiting: 'Editing',
    saving: 'Saving…',
    conflict: 'Not saved',
    failed: 'Not saved',
  }[state.status]
  return (
    <span className="kb-bar__state small faint" role="status">
      {text}
    </span>
  )
}

function Author({ page }: { page: Page }) {
  const agents = useAgents()
  const agent = agents.data?.find((item) => item.id === page.updated_by.agent)
  return (
    <p className="kb-author small faint">
      {agent && (
        <span {...agentHover(agent.id)} className="kb-author__cube">
          <Cube avatar={agent.avatar} size={16} />
        </span>
      )}
      Last edited by {page.updated_by.name}, {when(page.updated_at)}
    </p>
  )
}

const ICONS = [
  '📄',
  '📌',
  '📚',
  '🧭',
  '🛠️',
  '🧪',
  '💡',
  '✅',
  '🚀',
  '🔒',
  '🧩',
  '🗺️',
]

function IconPick({
  page,
  onPick,
}: {
  page: Page
  onPick: (icon: string) => void
}) {
  const [open, setOpen] = useState(false)
  return (
    <div className="kb-icon">
      <button
        type="button"
        className="kb-icon__button"
        aria-label={page.icon ? 'Change icon' : 'Add an icon'}
        data-empty={!page.icon}
        onClick={() => setOpen(!open)}
      >
        {page.icon ?? <Plus />}
      </button>
      {open && (
        <div className="kb-icon__list">
          {ICONS.map((icon) => (
            <button
              key={icon}
              type="button"
              className="kb-icon__choice"
              onClick={() => {
                onPick(icon)
                setOpen(false)
              }}
            >
              {icon}
            </button>
          ))}
          {page.icon && (
            <button
              type="button"
              className="kb-icon__choice"
              aria-label="Remove the icon"
              onClick={() => {
                onPick('')
                setOpen(false)
              }}
            >
              <X />
            </button>
          )}
        </div>
      )}
    </div>
  )
}

function useTitle(page: Page, onChange: (title: string) => void) {
  const [title, setTitle] = useState(page.title)
  const box = useRef<HTMLTextAreaElement>(null)
  useEffect(() => {
    const element = box.current
    if (!element) return
    element.style.height = 'auto'
    element.style.height = `${element.scrollHeight}px`
  }, [title])
  const props = {
    ref: box,
    className: 'kb-title',
    rows: 1,
    value: title,
    placeholder: page.kind === 'folder' ? 'Untitled folder' : 'Untitled',
    'aria-label': 'Title',
    onChange: (event: React.ChangeEvent<HTMLTextAreaElement>) => {
      const next = event.target.value.replaceAll('\n', ' ')
      setTitle(next)
      onChange(next)
    },
  }
  return { props, setTitle }
}

function Document({ page }: { page: Page }) {
  const client = useQueryClient()
  const navigate = useNavigate()
  const refreshTree = useRefreshTree()
  const save = useSave(page, refreshTree)
  const editor = useRef<EditorHandle>(null)
  const { follow } = save
  const { props: titleProps, setTitle } = useTitle(page, (next) =>
    save.change({ title: next }),
  )
  const [picking, setPicking] = useState(false)
  const [history, setHistory] = useState(false)
  const [problem, setProblem] = useState('')
  const uploads = useRef<HTMLInputElement>(null)
  const imageOnly = useRef(false)

  useEffect(() => {
    if (!follow(page.revision)) return
    setTitle(page.title)
    editor.current?.load(page.body)
  }, [page.revision, page.title, page.body, follow, setTitle])

  const refreshPage = useCallback(
    () => client.invalidateQueries({ queryKey: kbKeys.page(page.id) }),
    [client, page.id],
  )
  const attach = async (files: File[], inline = true) => {
    setProblem('')
    for (const file of files) {
      try {
        const attachment = await uploadAttachment(page.id, file)
        await refreshPage()
        if (!inline) continue
        const url = attachmentUrl(attachment.id)
        if (file.type.startsWith('image/'))
          editor.current?.image(file.name, url)
        else editor.current?.link(file.name, url)
      } catch (error) {
        setProblem(failure(error))
      }
    }
  }
  const actions = {
    pickImage: () => {
      imageOnly.current = true
      if (uploads.current) uploads.current.accept = 'image/*'
      uploads.current?.click()
    },
    pickFile: () => {
      imageOnly.current = false
      if (uploads.current) uploads.current.accept = ''
      uploads.current?.click()
    },
    pickPage: () => setPicking(true),
  }
  const openLink = (href: string) => {
    if (href.startsWith('/kb/')) {
      void navigate({ to: href })
      return
    }
    window.open(href, '_blank', 'noopener,noreferrer')
  }
  return (
    <div className="kb-doc-scroll">
      <Bar page={page} state={save.state} onHistory={() => setHistory(true)} />
      <article className="kb-doc">
        <IconPick
          page={page}
          onPick={(icon) => {
            void savePage(page.id, { icon }).then(async () => {
              await Promise.all([refreshTree(), refreshPage()])
            })
          }}
        />
        <textarea {...titleProps} />
        <Author page={page} />
        <SaveProblem
          state={save.state}
          onKeep={save.keepMine}
          onRetry={() => void save.retry()}
          onTheirs={async () => {
            const current = await client.fetchQuery({
              queryKey: kbKeys.page(page.id),
              queryFn: () =>
                ok(
                  api.GET('/api/kb/pages/{id}', {
                    params: { path: { id: page.id } },
                  }),
                ),
              staleTime: 0,
            })
            save.discard(current.revision)
            setTitle(current.title)
            editor.current?.load(current.body)
          }}
        />
        <Alert>{problem}</Alert>
        <Suspense fallback={<Skeleton lines={6} />}>
          <Editor
            ref={editor}
            markdown={page.body}
            actions={actions}
            onChange={(body) => save.change({ body })}
            onFiles={(files) => void attach(files)}
            onOpenLink={openLink}
          />
        </Suspense>
        <input
          ref={uploads}
          type="file"
          hidden
          multiple
          onChange={(event) => {
            void attach([...(event.currentTarget.files ?? [])])
            event.currentTarget.value = ''
          }}
        />
        <Attachments
          page={page}
          onAdd={actions.pickFile}
          onChanged={refreshPage}
        />
      </article>
      {picking && (
        <PagePick
          current={page.id}
          onClose={() => setPicking(false)}
          onPick={(target) => {
            setPicking(false)
            editor.current?.link(titled(target), `/kb/${target.id}`)
          }}
        />
      )}
      {history && (
        <Versions
          page={page}
          onClose={() => setHistory(false)}
          onRestored={async () => {
            setHistory(false)
            await refreshPage()
            await refreshTree()
          }}
        />
      )}
    </div>
  )
}

function SaveProblem({
  state,
  onKeep,
  onTheirs,
  onRetry,
}: {
  state: SaveState
  onKeep: () => void
  onTheirs: () => void
  onRetry: () => void
}) {
  if (state.status === 'failed')
    return (
      <div className="notice notice--danger kb-problem" role="alert">
        <span>Your changes are not saved yet. {state.message}</span>
        <button type="button" className="btn btn--sm" onClick={onRetry}>
          Try again
        </button>
      </div>
    )
  if (state.status !== 'conflict') return null
  return (
    <div className="notice notice--danger kb-problem" role="alert">
      <span>Someone else changed this page while you were writing.</span>
      <span className="actions">
        <button type="button" className="btn btn--sm" onClick={onTheirs}>
          Use their version
        </button>
        <button type="button" className="btn btn--sm" onClick={onKeep}>
          Keep mine
        </button>
      </span>
    </div>
  )
}

function size(bytes: number) {
  if (bytes < 1024) return `${bytes} B`
  if (bytes < 1024 * 1024) return `${Math.round(bytes / 1024)} KB`
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`
}

function Attachments({
  page,
  onAdd,
  onChanged,
}: {
  page: Page
  onAdd: () => void
  onChanged: () => Promise<unknown>
}) {
  const remove = async (attachment: Attachment) => {
    await ok(
      api.DELETE('/api/kb/attachments/{id}', {
        params: { path: { id: attachment.id } },
      }),
    )
    await onChanged()
  }
  return (
    <section className="kb-files" aria-label="Attachments">
      <header className="kb-files__head">
        <h2>Attachments</h2>
        <button
          type="button"
          className="btn btn--ghost btn--sm"
          onClick={onAdd}
        >
          <Paperclip /> Attach a file
        </button>
      </header>
      {page.attachments.map((attachment) => (
        <div key={attachment.id} className="kb-file">
          <FileIcon className="kb-row__icon" />
          <a href={attachmentUrl(attachment.id)} download={attachment.name}>
            {attachment.name}
          </a>
          <span className="small faint tabular">{size(attachment.size)}</span>
          <button
            type="button"
            className="btn btn--ghost btn--icon btn--xs"
            aria-label={`Remove ${attachment.name}`}
            onClick={() => void remove(attachment)}
          >
            <X />
          </button>
        </div>
      ))}
    </section>
  )
}

function PagePick({
  current,
  onClose,
  onPick,
}: {
  current: string
  onClose: () => void
  onPick: (page: { id: string; title: string; kind: string }) => void
}) {
  const tree = useTree()
  const [query, setQuery] = useState('')
  const pages = (tree.data ?? []).filter(
    (page) =>
      page.id !== current &&
      titled(page).toLowerCase().includes(query.toLowerCase()),
  )
  return (
    <Dialog open title="Link to a page" onClose={onClose}>
      <input
        autoFocus
        className="input"
        aria-label="Search pages"
        placeholder="Search pages"
        value={query}
        onChange={(event) => setQuery(event.target.value)}
      />
      <div className="kb-pick">
        {pages.map((page) => (
          <button
            key={page.id}
            type="button"
            className="row row--page"
            onClick={() => onPick(page)}
          >
            <PageIcon page={page} />
            <span className="row__label">{titled(page)}</span>
          </button>
        ))}
        {!pages.length && (
          <p className="kb-tree__none">No page matches that.</p>
        )}
      </div>
    </Dialog>
  )
}

function Versions({
  page,
  onClose,
  onRestored,
}: {
  page: Page
  onClose: () => void
  onRestored: () => Promise<unknown>
}) {
  const versions = useQuery({
    queryKey: ['kb', 'versions', page.id, page.revision],
    queryFn: () =>
      ok(
        api.GET('/api/kb/pages/{id}/versions', {
          params: { path: { id: page.id } },
        }),
      ),
  })
  const [revision, setRevision] = useState<number | null>(null)
  const [error, setError] = useState('')
  const chosen = revision ?? versions.data?.[0]?.revision ?? null
  const version = useQuery({
    queryKey: ['kb', 'version', page.id, chosen],
    enabled: chosen !== null,
    queryFn: () =>
      ok(
        api.GET('/api/kb/pages/{id}/versions/{revision}', {
          params: { path: { id: page.id, revision: chosen ?? 0 } },
        }),
      ),
  })
  const restore = async () => {
    if (!version.data) return
    setError('')
    try {
      await savePage(page.id, {
        expected_revision: page.revision,
        title: version.data.title,
        body: version.data.body,
      })
      await onRestored()
    } catch (error) {
      setError(failure(error))
    }
  }
  return (
    <Dialog open title="Version history" onClose={onClose}>
      <Alert>{error}</Alert>
      {versions.data?.length === 0 && (
        <Empty title="No earlier versions yet.">
          A page gains a version each time someone new edits it, and every few
          minutes while they keep going.
        </Empty>
      )}
      <div className="kb-versions">
        <ul className="kb-versions__list">
          {versions.data?.map((item) => (
            <li key={item.revision}>
              <button
                type="button"
                className="kb-version"
                aria-pressed={item.revision === chosen}
                onClick={() => setRevision(item.revision)}
              >
                <strong>{when(item.saved_at)}</strong>
                <span className="small faint">{item.by.name}</span>
              </button>
            </li>
          ))}
        </ul>
        {version.data && (
          <div className="kb-versions__preview">
            <h3>{version.data.title || 'Untitled'}</h3>
            <Suspense fallback={<Skeleton lines={4} />}>
              <Editor
                key={version.data.revision}
                markdown={version.data.body}
                editable={false}
                onOpenLink={() => undefined}
              />
            </Suspense>
          </div>
        )}
      </div>
      {version.data && (
        <div className="actions">
          <button
            type="button"
            className="btn btn--primary"
            disabled={version.data.revision === page.revision}
            onClick={() => void restore()}
          >
            Restore this version
          </button>
        </div>
      )}
    </Dialog>
  )
}

function FolderView({ page }: { page: Page }) {
  const tree = useTree()
  const refreshTree = useRefreshTree()
  const save = useSave(page, refreshTree)
  const { props: titleProps } = useTitle(page, (next) =>
    save.change({ title: next }),
  )
  const { create, error } = useCreate()
  const children = childrenOf(tree.data ?? []).get(page.id) ?? []
  return (
    <div className="kb-doc-scroll">
      <Bar page={page} state={save.state} />
      <article className="kb-doc">
        <textarea {...titleProps} />
        <div className="actions">
          <button
            type="button"
            className="btn btn--primary btn--sm"
            onClick={() => void create('page', page.id)}
          >
            <Plus /> New page
          </button>
          <button
            type="button"
            className="btn btn--sm"
            onClick={() => void create('folder', page.id)}
          >
            <FolderPlus /> New folder
          </button>
        </div>
        <Alert>{error}</Alert>
        <ul className="kb-children">
          {children.map((child) => (
            <li key={child.id}>
              <Link
                to="/kb/$pageId"
                params={{ pageId: child.id }}
                className="kb-child"
              >
                <PageIcon page={child} />
                <span className="row__label">{titled(child)}</span>
                <span className="small faint">
                  {child.updated_by.name}, {when(child.updated_at)}
                </span>
              </Link>
            </li>
          ))}
        </ul>
        {!children.length && <p className="muted">This folder is empty.</p>}
      </article>
    </div>
  )
}

export function Home() {
  const tree = useTree()
  const { create, error } = useCreate()
  const pages = tree.data ?? []
  const recent = pages
    .filter((page) => page.kind === 'page')
    .sort((a, b) => b.updated_at.localeCompare(a.updated_at))
    .slice(0, 8)
  if (tree.isPending) return <Skeleton lines={5} width="min(720px, 100%)" />
  return (
    <div className="kb-doc-scroll">
      <article className="kb-doc kb-home">
        <h1>Knowledge base</h1>
        <p className="muted">
          One wiki for this whole installation. People and agents read it, write
          it and link it together, and pages stay when conversations end.
        </p>
        <div className="actions">
          <button
            type="button"
            className="btn btn--primary"
            onClick={() => void create('page', null)}
          >
            <Plus /> New page
          </button>
          <button
            type="button"
            className="btn"
            onClick={() => void create('folder', null)}
          >
            <FolderPlus /> New folder
          </button>
        </div>
        <Alert>{error}</Alert>
        {recent.length > 0 && (
          <section className="kb-recent">
            <h2>Recently edited</h2>
            <ul className="kb-children">
              {recent.map((page) => (
                <li key={page.id}>
                  <Link
                    to="/kb/$pageId"
                    params={{ pageId: page.id }}
                    className="kb-child"
                  >
                    <PageIcon page={page} />
                    <span className="row__label">{titled(page)}</span>
                    <span className="small faint">
                      {page.updated_by.name}, {when(page.updated_at)}
                    </span>
                  </Link>
                </li>
              ))}
            </ul>
          </section>
        )}
      </article>
    </div>
  )
}
