import { useQuery, useQueryClient } from '@tanstack/react-query'
import { Link, useNavigate } from '@tanstack/react-router'
import {
  ChevronRight,
  File,
  Folder,
  FolderPlus,
  MoreHorizontal,
  Plus,
  Search,
  Trash2,
} from 'lucide-react'
import { useEffect, useState, type ReactNode } from 'react'
import { api, failure, ok } from '../../api/client'
import {
  Alert,
  Dialog,
  Empty,
  Menu,
  Skeleton,
  when,
} from '../../components/primitives'
import {
  createPage,
  kbKeys,
  placePage,
  trashPage,
  useRefreshTree,
  useTree,
} from './api'
import {
  childrenOf,
  dropTarget,
  pathTo,
  titled,
  under,
  type Drop,
  type Summary,
} from './tree'
import './kb.css'

const OPEN_KEY = 'cubebot.kb.open'

function storedOpen(): string[] {
  try {
    const stored: unknown = JSON.parse(localStorage.getItem(OPEN_KEY) ?? '[]')
    return Array.isArray(stored) ? (stored as string[]) : []
  } catch {
    return []
  }
}

/** Which rows are open, per browser: a convenience, not worth a server round trip. */
function useOpenRows(reveal: string[]) {
  const [open, setOpen] = useState(() => new Set(storedOpen()))
  // A page opened by link or search shows where it lives, once, so it can still be collapsed.
  const wanted = reveal.join()
  const [revealed, setRevealed] = useState('')
  if (wanted !== revealed) {
    setRevealed(wanted)
    setOpen(new Set([...open, ...reveal]))
  }
  const toggle = (id: string, to?: boolean) =>
    setOpen((current) => {
      const next = new Set(current)
      if (to ?? !next.has(id)) next.add(id)
      else next.delete(id)
      try {
        localStorage.setItem(OPEN_KEY, JSON.stringify([...next]))
      } catch {
        // Private windows only lose the convenience.
      }
      return next
    })
  return { open, toggle }
}

export function PageIcon({ page }: { page: Pick<Summary, 'icon' | 'kind'> }) {
  if (page.icon) return <span className="kb-emoji">{page.icon}</span>
  return page.kind === 'folder' ? (
    <Folder className="kb-row__icon" />
  ) : (
    <File className="kb-row__icon" />
  )
}

/** Creates a page or folder and opens it. */
export function useCreate() {
  const navigate = useNavigate()
  const refresh = useRefreshTree()
  const [error, setError] = useState('')
  const create = async (kind: 'page' | 'folder', parent: string | null) => {
    setError('')
    try {
      const page = await createPage(kind, parent)
      await refresh()
      await navigate({ to: '/kb/$pageId', params: { pageId: page.id } })
    } catch (error) {
      setError(failure(error))
    }
  }
  return { create, error }
}

/** The tree beside every page: the page itself on phones. */
export function KnowledgeTree({ current }: { current?: string | undefined }) {
  const tree = useTree()
  const { create, error } = useCreate()
  const [query, setQuery] = useState('')
  const [trash, setTrash] = useState(false)
  const pages = tree.data ?? []
  return (
    <aside className="kb-tree" aria-label="Knowledge base">
      <header className="kb-tree__head">
        <div className="kb-search">
          <Search aria-hidden="true" />
          <input
            className="input"
            type="search"
            aria-label="Search the knowledge base"
            placeholder="Search"
            value={query}
            onChange={(event) => setQuery(event.target.value)}
          />
        </div>
        <Menu
          className="menu--right"
          trigger={<Plus />}
          triggerClassName="btn btn--ghost btn--icon btn--sm"
          label="New"
        >
          <button
            type="button"
            className="menu__item"
            onClick={() => void create('page', null)}
          >
            <File /> New page
          </button>
          <button
            type="button"
            className="menu__item"
            onClick={() => void create('folder', null)}
          >
            <FolderPlus /> New folder
          </button>
        </Menu>
      </header>
      <Alert>{error}</Alert>
      <div className="kb-tree__scroll">
        {query.trim() ? (
          <Results query={query.trim()} />
        ) : tree.isPending ? (
          <Skeleton lines={5} />
        ) : (
          <Rows pages={pages} current={current} />
        )}
      </div>
      <footer className="kb-tree__foot">
        <button
          type="button"
          className="row row--page"
          onClick={() => setTrash(true)}
        >
          <Trash2 className="row__icon" />
          <span className="row__label">Trash</span>
        </button>
      </footer>
      {trash && <Trash onClose={() => setTrash(false)} />}
    </aside>
  )
}

function Results({ query }: { query: string }) {
  const [settled, setSettled] = useState(query)
  useEffect(() => {
    const wait = window.setTimeout(() => setSettled(query), 200)
    return () => window.clearTimeout(wait)
  }, [query])
  const hits = useQuery({
    queryKey: ['kb', 'search', settled],
    queryFn: () =>
      ok(api.GET('/api/kb/search', { params: { query: { q: settled } } })),
  })
  if (hits.isPending) return <Skeleton lines={3} />
  if (!hits.data?.length)
    return <p className="kb-tree__none">Nothing matches that.</p>
  return (
    <ul className="kb-hits">
      {hits.data.map((hit) => (
        <li key={hit.id}>
          <Link
            to="/kb/$pageId"
            params={{ pageId: hit.id }}
            className="kb-hit"
            activeProps={{ 'aria-current': 'page' }}
          >
            <span className="kb-hit__title">
              <PageIcon page={hit} />
              {titled(hit)}
            </span>
            {hit.snippet && (
              <span className="kb-hit__snippet">{hit.snippet}</span>
            )}
          </Link>
        </li>
      ))}
    </ul>
  )
}

/** Every row, with what a drag is doing, which only the whole tree can tell. */
function Rows({
  pages,
  current,
}: {
  pages: Summary[]
  current: string | undefined
}) {
  const above = pathTo(pages, current ?? '')
    .slice(0, -1)
    .map((page) => page.id)
  const { open, toggle } = useOpenRows(above)
  const [moving, setMoving] = useState<Summary | null>(null)
  const refresh = useRefreshTree()
  const [error, setError] = useState('')
  const children = childrenOf(pages)
  const place = async (id: string, target: Parameters<typeof placePage>[1]) => {
    setError('')
    try {
      await placePage(id, target)
      if (target.parent) toggle(target.parent, true)
      await refresh()
    } catch (error) {
      setError(failure(error))
    }
  }
  if (!pages.length)
    return (
      <p className="kb-tree__none">
        Pages you and your agents write appear here.
      </p>
    )
  const row = (page: Summary, depth: number): ReactNode => (
    <TreeRow
      key={page.id}
      page={page}
      pages={pages}
      depth={depth}
      current={current}
      open={open.has(page.id)}
      hasChildren={Boolean(children.get(page.id)?.length)}
      onToggle={() => toggle(page.id)}
      onDrop={(id, target) => void place(id, target)}
      onMove={() => setMoving(page)}
    >
      {open.has(page.id) &&
        children.get(page.id)?.map((child) => row(child, depth + 1))}
    </TreeRow>
  )
  return (
    <>
      <Alert>{error}</Alert>
      <ul className="kb-rows" role="tree" aria-label="Pages">
        {children.get('')?.map((page) => row(page, 0))}
      </ul>
      {moving && (
        <MoveDialog
          page={moving}
          pages={pages}
          onClose={() => setMoving(null)}
          onPick={(parent) => {
            const last = children.get(parent ?? '')?.at(-1)?.id ?? null
            void place(moving.id, {
              parent,
              after: last === moving.id ? null : last,
            })
            setMoving(null)
          }}
        />
      )}
    </>
  )
}

function dropAt(event: React.DragEvent): Drop {
  const box = event.currentTarget.getBoundingClientRect()
  const at = (event.clientY - box.top) / box.height
  if (at < 0.25) return 'before'
  return at > 0.75 ? 'after' : 'inside'
}

function TreeRow({
  page,
  pages,
  depth,
  current,
  open,
  hasChildren,
  onToggle,
  onDrop,
  onMove,
  children,
}: {
  page: Summary
  pages: Summary[]
  depth: number
  current: string | undefined
  open: boolean
  hasChildren: boolean
  onToggle: () => void
  onDrop: (
    id: string,
    target: { parent: string | null; after: string | null },
  ) => void
  onMove: () => void
  children: ReactNode
}) {
  const [over, setOver] = useState<Drop | null>(null)
  const { create } = useCreate()
  const refresh = useRefreshTree()
  const navigate = useNavigate()
  return (
    <li
      role="treeitem"
      aria-expanded={hasChildren ? open : undefined}
      aria-selected={page.id === current}
    >
      <div
        className="kb-row"
        data-drop={over}
        style={{ paddingLeft: 4 + depth * 14 }}
        draggable
        onDragStart={(event) => {
          event.dataTransfer.setData('application/x-cubebot-page', page.id)
          event.dataTransfer.effectAllowed = 'move'
        }}
        onDragOver={(event) => {
          if (!event.dataTransfer.types.includes('application/x-cubebot-page'))
            return
          event.preventDefault()
          setOver(dropAt(event))
        }}
        onDragLeave={() => setOver(null)}
        onDrop={(event) => {
          event.preventDefault()
          setOver(null)
          const moved = event.dataTransfer.getData('application/x-cubebot-page')
          const target = dropTarget(pages, moved, page, dropAt(event))
          if (target) onDrop(moved, target)
        }}
      >
        <button
          type="button"
          className="kb-row__chevron"
          aria-label={open ? 'Collapse' : 'Expand'}
          data-open={open}
          data-empty={!hasChildren}
          onClick={onToggle}
        >
          <ChevronRight />
        </button>
        <Link
          to="/kb/$pageId"
          params={{ pageId: page.id }}
          className="kb-row__link"
          activeProps={{ 'aria-current': 'page' }}
        >
          <PageIcon page={page} />
          <span className="row__label">{titled(page)}</span>
        </Link>
        <span className="kb-row__actions">
          <Menu
            className="menu--right"
            trigger={<MoreHorizontal />}
            triggerClassName="btn btn--ghost btn--icon btn--xs"
            label={`More for ${titled(page)}`}
          >
            <button type="button" className="menu__item" onClick={onMove}>
              Move to…
            </button>
            <button
              type="button"
              className="menu__item menu__item--danger"
              onClick={async () => {
                await trashPage(page.id)
                await refresh()
                if (page.id === current) await navigate({ to: '/kb' })
              }}
            >
              Move to trash
            </button>
          </Menu>
          <button
            type="button"
            className="btn btn--ghost btn--icon btn--xs"
            aria-label={`New page in ${titled(page)}`}
            title="New page inside"
            onClick={() => {
              if (!open) onToggle()
              void create('page', page.id)
            }}
          >
            <Plus />
          </button>
        </span>
      </div>
      {children && <ul role="group">{children}</ul>}
    </li>
  )
}

/** Moving without dragging, which touch screens need. */
function MoveDialog({
  page,
  pages,
  onClose,
  onPick,
}: {
  page: Summary
  pages: Summary[]
  onClose: () => void
  onPick: (parent: string | null) => void
}) {
  const blocked = under(pages, page.id)
  const choices = pages.filter(
    (item) => item.id !== page.id && !blocked.has(item.id),
  )
  return (
    <Dialog open title={`Move ${titled(page)}`} onClose={onClose}>
      <div className="kb-pick">
        <button
          type="button"
          className="row row--page"
          onClick={() => onPick(null)}
        >
          <span className="row__label">Top level</span>
        </button>
        {choices.map((item) => (
          <button
            key={item.id}
            type="button"
            className="row row--page"
            onClick={() => onPick(item.id)}
          >
            <PageIcon page={item} />
            <span className="row__label">
              {pathTo(pages, item.id).map(titled).join(' / ')}
            </span>
          </button>
        ))}
      </div>
    </Dialog>
  )
}

/** What was thrown away: restore it, or delete it for good. */
function Trash({ onClose }: { onClose: () => void }) {
  const client = useQueryClient()
  const refresh = useRefreshTree()
  const trash = useQuery({
    queryKey: kbKeys.trash,
    queryFn: () => ok(api.GET('/api/kb/trash')),
    staleTime: 0,
  })
  const [error, setError] = useState('')
  const act = async (run: () => Promise<unknown>) => {
    setError('')
    try {
      await run()
      await Promise.all([
        refresh(),
        client.invalidateQueries({ queryKey: kbKeys.trash }),
      ])
    } catch (error) {
      setError(failure(error))
    }
  }
  return (
    <Dialog open title="Trash" onClose={onClose}>
      <Alert>{error}</Alert>
      {trash.isPending && <Skeleton lines={2} />}
      {trash.data?.length === 0 && (
        <Empty title="The trash is empty.">
          Anything you or an agent throws away waits here until someone deletes
          it for good.
        </Empty>
      )}
      <ul className="kb-trash">
        {trash.data?.map((page) => (
          <li key={page.id} className="kb-trash__row">
            <PageIcon page={page} />
            <span className="kb-trash__name">
              <span className="row__label">{titled(page)}</span>
              <span className="small faint">
                Last edited by {page.updated_by.name}, {when(page.updated_at)}
              </span>
            </span>
            <button
              type="button"
              className="btn btn--sm"
              onClick={() =>
                void act(() =>
                  ok(
                    api.POST('/api/kb/pages/{id}/restore', {
                      params: { path: { id: page.id } },
                    }),
                  ),
                )
              }
            >
              Restore
            </button>
            <button
              type="button"
              className="btn btn--sm btn--danger"
              onClick={() => {
                if (
                  !window.confirm(
                    `Delete ${titled(page)} for good, with everything under it?`,
                  )
                )
                  return
                void act(() =>
                  ok(
                    api.POST('/api/kb/pages/{id}/purge', {
                      params: { path: { id: page.id } },
                    }),
                  ),
                )
              }}
            >
              Delete
            </button>
          </li>
        ))}
      </ul>
    </Dialog>
  )
}
