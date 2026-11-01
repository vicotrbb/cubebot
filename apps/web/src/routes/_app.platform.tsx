import { createFileRoute, Link, Outlet } from '@tanstack/react-router'
import { useSession } from '../app/queries'
import { Empty } from '../components/primitives'
import { PLATFORM } from '../features/shell/Sidebar'

export const Route = createFileRoute('/_app/platform')({ component: Platform })

/** Installation administration: dense tables with an editor beside them. */
function Platform() {
  const session = useSession()
  if (session.data && !session.data.administrator)
    return <Empty title="Platform settings are for administrators." />
  return (
    <div className="page">
      <header className="page__head">
        <h1>Platform</h1>
        <nav className="tabs" aria-label="Platform">
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
