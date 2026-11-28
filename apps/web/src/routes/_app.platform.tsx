import {
  createFileRoute,
  Link,
  Outlet,
  useRouterState,
} from '@tanstack/react-router'
import { useEffect, useRef } from 'react'
import { useSession } from '../app/queries'
import { Empty } from '../components/primitives'
import { PLATFORM } from '../features/shell/Sidebar'

export const Route = createFileRoute('/_app/platform')({ component: Platform })

/** Auditors read these pages; the rest configure the installation. */
const AUDITED = new Set<string>([
  '/platform/people',
  '/platform/groups',
  '/platform/ownership',
  '/platform/audit',
  '/platform/health',
])

function Platform() {
  const session = useSession()
  const nav = useRef<HTMLElement>(null)
  const path = useRouterState({ select: (state) => state.location.pathname })
  useEffect(() => {
    nav.current
      ?.querySelector('[aria-current="page"]')
      ?.scrollIntoView({ block: 'nearest', inline: 'nearest' })
  }, [path])
  const role = session.data?.role
  if (session.data && role !== 'administrator' && role !== 'auditor')
    return <Empty title="Platform settings are for administrators." />
  const auditor = role === 'auditor'
  const tabs = auditor ? PLATFORM.filter(([to]) => AUDITED.has(to)) : PLATFORM
  return (
    <div className="page">
      <header className="page__head">
        <h1>Platform</h1>
        <nav ref={nav} className="tabs page__tabs" aria-label="Platform">
          {tabs.map(([to, label]) => (
            <Link key={to} to={to} activeProps={{ 'aria-current': 'page' }}>
              {label}
            </Link>
          ))}
        </nav>
      </header>
      <div className="page__body">
        {auditor && !AUDITED.has(path.replace(/\/$/, '')) ? (
          <Empty title="This page is for administrators.">
            As an auditor you can read People, Groups and access, Ownership, the
            Audit log and Health.
          </Empty>
        ) : (
          <Outlet />
        )}
      </div>
    </div>
  )
}
