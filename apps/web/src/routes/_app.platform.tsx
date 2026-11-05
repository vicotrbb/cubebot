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

function Platform() {
  const session = useSession()
  const nav = useRef<HTMLElement>(null)
  const path = useRouterState({ select: (state) => state.location.pathname })
  useEffect(() => {
    nav.current
      ?.querySelector('[aria-current="page"]')
      ?.scrollIntoView({ block: 'nearest', inline: 'nearest' })
  }, [path])
  if (session.data && !session.data.administrator)
    return <Empty title="Platform settings are for administrators." />
  return (
    <div className="page">
      <header className="page__head">
        <h1>Platform</h1>
        <nav ref={nav} className="tabs page__tabs" aria-label="Platform">
          {PLATFORM.map(([to, label]) => (
            <Link key={to} to={to} activeProps={{ 'aria-current': 'page' }}>
              {label}
            </Link>
          ))}
        </nav>
      </header>
      <div className="page__body">
        <Outlet />
      </div>
    </div>
  )
}
