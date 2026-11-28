import { useInfiniteQuery } from '@tanstack/react-query'
import { createFileRoute } from '@tanstack/react-router'
import { ChevronRight, Download, ShieldAlert } from 'lucide-react'
import { Fragment, useState } from 'react'
import { api, failure, ok } from '../api/client'
import type { Schemas } from '../api/types'
import { useAgents } from '../app/queries'
import { Cube, moodOf } from '../components/Cube'
import { PersonAvatar } from '../components/PersonAvatar'
import { Alert, Empty, Skeleton, Status } from '../components/primitives'
import {
  day,
  FILTERS,
  isSelf,
  phrase,
  target,
  type Detail,
} from '../features/platform/audit'
import { auditCsv } from '../features/platform/csv'
import '../features/platform/platform.css'

export const Route = createFileRoute('/_app/platform/audit')({
  component: Audit,
})

const PAGE = 100

const MOST = 10_000

type AuditEntry = Schemas['AuditEntry']

/** One page of the log, newest first, older than `before` when given. */
const auditPage = (filter: string, before?: number) =>
  ok(
    api.GET('/api/audit', {
      params: {
        query: {
          ...(before === undefined ? {} : { before }),
          ...(filter ? { area: filter } : {}),
        },
      },
    }),
  )

async function exportAll(filter: string, onCount: (count: number) => void) {
  const entries: AuditEntry[] = []
  for (;;) {
    const page = await auditPage(filter, entries.at(-1)?.sequence)
    entries.push(...page)
    onCount(entries.length)
    if (page.length < PAGE || entries.length >= MOST)
      return entries.slice(0, MOST)
  }
}

function download(text: string, name: string) {
  // The byte-order mark makes spreadsheet apps read the file as UTF-8.
  const url = URL.createObjectURL(
    new Blob(['\uFEFF', text], { type: 'text/csv;charset=utf-8' }),
  )
  const link = document.createElement('a')
  link.href = url
  link.download = name
  link.click()
  window.setTimeout(() => URL.revokeObjectURL(url), 1000)
}

function ExportCsv({ filter, label }: { filter: string; label: string }) {
  const [count, setCount] = useState<number | null>(null)
  const [error, setError] = useState('')
  const [notice, setNotice] = useState('')
  const run = async () => {
    setError('')
    setNotice('')
    setCount(0)
    try {
      const entries = await exportAll(filter, setCount)
      const day = new Date().toISOString().slice(0, 10)
      const area = filter
        ? `-${label.toLowerCase().replaceAll(/\W+/g, '-')}`
        : ''
      download(auditCsv(entries), `cubebot-audit${area}-${day}.csv`)
      setNotice(
        entries.length >= MOST
          ? `Exported the newest ${MOST.toLocaleString()} entries, the most one file holds.`
          : `Exported ${entries.length.toLocaleString()} entries.`,
      )
    } catch (caught) {
      setError(failure(caught))
    } finally {
      setCount(null)
    }
  }
  return (
    <div className="audit__export">
      <button
        type="button"
        className="btn btn--sm"
        disabled={count !== null}
        onClick={() => void run()}
      >
        <Download aria-hidden="true" />
        {count === null ? 'Export CSV' : `Exporting… ${count.toLocaleString()}`}
      </button>
      <Alert>{error}</Alert>
      <Status>{notice}</Status>
    </div>
  )
}

function Audit() {
  const [filter, setFilter] = useState<string>(FILTERS[0].areas)
  const log = useInfiniteQuery({
    queryKey: ['audit', filter],
    initialPageParam: undefined as number | undefined,
    queryFn: ({ pageParam }) => auditPage(filter, pageParam),
    getNextPageParam: (last) =>
      last.length === PAGE ? last.at(-1)?.sequence : undefined,
  })
  const entries = log.data?.pages.flat() ?? []
  return (
    <div className="audit">
      <div className="audit__bar">
        <p className="platform__lead">
          Who did what, newest first. Passwords, tokens and vault secrets never
          appear here.
        </p>
        <nav className="tabs" aria-label="Filter the audit log">
          {FILTERS.map((item) => (
            <button
              key={item.label}
              type="button"
              aria-pressed={filter === item.areas}
              onClick={() => setFilter(item.areas)}
            >
              {item.label}
            </button>
          ))}
        </nav>
        <ExportCsv
          filter={filter}
          label={FILTERS.find((item) => item.areas === filter)?.label ?? ''}
        />
      </div>
      {log.isPending && <Skeleton lines={6} />}
      {log.isError && <Alert>The audit log could not be loaded.</Alert>}
      {log.data && entries.length === 0 && (
        <Empty title="Nothing recorded here yet.">
          Changes and sign-ins appear the moment they happen.
        </Empty>
      )}
      <ol className="audit__list">
        {entries.map((entry, index) => {
          const label = day(entry.at)
          const previous = entries[index - 1]
          const first = !previous || day(previous.at) !== label
          return (
            <Fragment key={entry.sequence}>
              {first && (
                <li className="audit__day" aria-hidden="true">
                  {label}
                </li>
              )}
              <Entry entry={entry} />
            </Fragment>
          )
        })}
      </ol>
      {log.hasNextPage && (
        <div className="actions audit__more">
          <button
            type="button"
            className="btn btn--sm"
            disabled={log.isFetchingNextPage}
            onClick={() => void log.fetchNextPage()}
          >
            {log.isFetchingNextPage ? 'Loading…' : 'Load older'}
          </button>
        </div>
      )}
    </div>
  )
}

function Entry({ entry }: { entry: AuditEntry }) {
  const [open, setOpen] = useState(false)
  const detail = entry.detail as Detail
  const self = isSelf(entry.kind, entry.actor, entry.resource)
  const name = self
    ? null
    : target(entry.kind, detail, entry.resource_name, entry.resource)
  const keys = Object.entries(detail).filter(
    ([, value]) => value !== null && value !== '',
  )
  const at = new Date(entry.at)
  return (
    <li className="audit__entry" data-open={open}>
      <Actor entry={entry} />
      <p className="audit__sentence">
        <strong>{actorName(entry)}</strong> {phrase(entry.kind, detail, self)}
        {name && (
          <>
            {' '}
            <strong>{name}</strong>
          </>
        )}
      </p>
      <time
        className="audit__time tabular"
        dateTime={entry.at}
        title={at.toLocaleString()}
      >
        {at.toLocaleTimeString(undefined, {
          hour: '2-digit',
          minute: '2-digit',
        })}
      </time>
      <button
        type="button"
        className="btn btn--ghost btn--icon btn--sm audit__toggle"
        aria-expanded={open}
        aria-label={open ? 'Hide details' : 'Show details'}
        onClick={() => setOpen(!open)}
      >
        <ChevronRight />
      </button>
      {open && (
        <dl className="kv audit__detail">
          <dt>Event</dt>
          <dd>
            <code>{entry.kind}</code> · #{entry.sequence}
          </dd>
          {entry.resource && (
            <>
              <dt>Resource</dt>
              <dd>
                <code>{entry.resource}</code>
              </dd>
            </>
          )}
          {keys.map(([key, value]) => (
            <Fragment key={key}>
              <dt>{key.replaceAll('_', ' ')}</dt>
              <dd>
                {typeof value === 'object' ? (
                  <code>{JSON.stringify(value)}</code>
                ) : (
                  String(value)
                )}
              </dd>
            </Fragment>
          ))}
        </dl>
      )}
    </li>
  )
}

function actorName(entry: AuditEntry) {
  const nobody = entry.kind.startsWith('watchdog.') ? 'The watchdog' : 'Someone'
  return entry.agent_name ?? entry.actor_name ?? nobody
}

function Actor({ entry }: { entry: AuditEntry }) {
  const agents = useAgents()
  if (entry.agent) {
    const agent = agents.data?.find((item) => item.id === entry.agent)
    return (
      <Cube
        agentId={agent?.id}
        avatar={agent?.avatar ?? null}
        mood={agent ? moodOf(agent) : 'idle'}
        size={24}
        className="audit__who"
      />
    )
  }
  if (entry.actor)
    return (
      <PersonAvatar
        account={entry.actor}
        name={entry.actor_name ?? '?'}
        className="audit__who"
      />
    )
  return (
    <span
      className="avatar-initial audit__who audit__who--none"
      aria-hidden="true"
    >
      <ShieldAlert size={14} />
    </span>
  )
}
