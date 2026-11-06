import { useQuery } from '@tanstack/react-query'
import { Search } from 'lucide-react'
import { useEffect, useMemo, useState } from 'react'
import { api, failure } from '../../api/client'
import { Alert, Skeleton, Status, when } from '../../components/primitives'
import { answer, CATALOG_KEY, CONNECTIONS_KEY } from './api'
import { Mark } from './Mark'
import {
  CATEGORIES,
  type Category,
  type Connection,
  type Manifest,
  categories,
  matches,
  standing,
} from './model'
import { Sheet } from './Sheet'
import './connections.css'

const RETURNED: Record<string, string> = {
  expired: 'That sign-in expired before it came back. Start it again.',
  signed_out: 'Sign in to the console in this browser, then start again.',
  refused:
    'The vendor refused the sign-in. Check the app’s settings and start again.',
  unexpected: 'The sign-in came back without what it needed. Start it again.',
}

export function ConnectionsPage() {
  const catalog = useQuery({
    queryKey: CATALOG_KEY,
    queryFn: () => answer(api.GET('/api/connectors')),
    staleTime: Infinity,
  })
  const connections = useQuery({
    queryKey: CONNECTIONS_KEY,
    queryFn: () => answer(api.GET('/api/connections')),
    // Health checks and renewals change status without anyone touching this page.
    refetchInterval: 30_000,
  })
  const [query, setQuery] = useState('')
  const [category, setCategory] = useState<Category | null>(null)
  const [open, setOpen] = useState<{ id: string; account?: string } | null>(
    null,
  )
  const [returned, setReturned] = useState({ text: '', failed: false })
  useReturn(connections.data ?? null, setOpen, setReturned)

  const connectors = catalog.data?.connectors ?? []
  const byConnector = useMemo(() => {
    const grouped = new Map<string, Connection[]>()
    for (const c of connections.data ?? [])
      grouped.set(c.connector, [...(grouped.get(c.connector) ?? []), c])
    return grouped
  }, [connections.data])
  const shown = connectors.filter((m) => matches(m, query, category))
  const opened = connectors.find((m) => m.id === open?.id)

  return (
    <div className="connectors">
      <p className="platform__lead">
        Accounts your agents act through, such as a mailbox, a calendar or a
        repository. Each signs in with an app you own; agents find its tools
        when they need them and never see its password or token.
      </p>
      <Notices returned={returned} error={catalog.error ?? connections.error} />
      <Toolbar
        connectors={connectors}
        query={query}
        category={category}
        onQuery={setQuery}
        onCategory={setCategory}
      />
      {catalog.isPending && <Skeleton lines={4} />}
      {catalog.data && shown.length === 0 && (
        <p className="connectors__none">
          Nothing matches “{query}”. Try a product such as Gmail, or the generic
          HTTP API for any other service.
        </p>
      )}
      <ul className="connectors__grid" aria-label="Connectors">
        {shown.map((manifest) => (
          <Card
            key={manifest.id}
            manifest={manifest}
            connections={byConnector.get(manifest.id) ?? []}
            onOpen={() => setOpen({ id: manifest.id })}
          />
        ))}
      </ul>
      {opened && catalog.data && (
        <Sheet
          catalog={catalog.data}
          manifest={opened}
          connections={byConnector.get(opened.id) ?? []}
          all={connections.data ?? []}
          focus={open?.account}
          onClose={() => setOpen(null)}
        />
      )}
    </div>
  )
}

function Notices({
  returned,
  error,
}: {
  returned: { text: string; failed: boolean }
  error: unknown
}) {
  return (
    <>
      <Status>{returned.failed ? '' : returned.text}</Status>
      <Alert>{returned.failed ? returned.text : ''}</Alert>
      <Alert>{error ? failure(error) : ''}</Alert>
    </>
  )
}

function Toolbar({
  connectors,
  query,
  category,
  onQuery,
  onCategory,
}: {
  connectors: Manifest[]
  query: string
  category: Category | null
  onQuery: (query: string) => void
  onCategory: (category: Category | null) => void
}) {
  return (
    <div className="connectors__bar">
      <label className="connectors__search">
        <Search aria-hidden="true" />
        <input
          type="search"
          placeholder="Search connectors and tools"
          aria-label="Search connectors and tools"
          value={query}
          onChange={(event) => onQuery(event.target.value)}
        />
      </label>
      <div
        className="tabs connectors__filters"
        role="group"
        aria-label="Categories"
      >
        <button
          type="button"
          aria-pressed={category === null}
          onClick={() => onCategory(null)}
        >
          All
        </button>
        {categories(connectors).map((id) => (
          <button
            key={id}
            type="button"
            aria-pressed={category === id}
            onClick={() => onCategory(category === id ? null : id)}
          >
            {CATEGORIES[id]}
          </button>
        ))}
      </div>
    </div>
  )
}

function Card({
  manifest,
  connections,
  onOpen,
}: {
  manifest: Manifest
  connections: Connection[]
  onOpen: () => void
}) {
  const { tone, text } = standing(connections, when)
  return (
    <li>
      <button
        type="button"
        className="connector"
        data-tone={tone}
        onClick={onOpen}
      >
        <span className="connector__head">
          <Mark id={manifest.id} />
          <span className="connector__name">{manifest.name}</span>
        </span>
        <span className="connector__description">{manifest.description}</span>
        <span className="connector__standing">
          {tone !== 'none' && (
            <span className="connector__dot" aria-hidden="true" />
          )}
          <span className="connector__status">{text}</span>
        </span>
      </button>
    </li>
  )
}

/** A vendor that sent the browser back here names the connection it saved, or why it did not. */
function useReturn(
  connections: Connection[] | null,
  setOpen: (open: { id: string; account: string }) => void,
  setReturned: (returned: { text: string; failed: boolean }) => void,
) {
  useEffect(() => {
    if (!connections) return
    const search = new URLSearchParams(window.location.search)
    const connected = search.get('connected')
    const problem = search.get('sign_in_error')
    if (!connected && !problem) return
    window.history.replaceState(null, '', window.location.pathname)
    const saved = connections.find((c) => c.id === connected)
    if (saved) {
      setOpen({ id: saved.connector, account: saved.id })
      setReturned({
        text: `Signed in as ${saved.account ?? saved.slug}.`,
        failed: false,
      })
    }
    if (problem)
      setReturned({
        text: RETURNED[problem] ?? RETURNED.unexpected ?? '',
        failed: true,
      })
  }, [connections, setOpen, setReturned])
}
