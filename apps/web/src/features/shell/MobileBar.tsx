import { Link } from '@tanstack/react-router'
import { Folder, House, Inbox, UserRound } from 'lucide-react'
import { useActivity } from '../../app/queries'

export function MobileBar() {
  const activity = useActivity()
  const attention = activity.data?.length ?? 0
  return (
    <nav className="bottombar" aria-label="Main">
      <Link
        to="/"
        className="bottombar__item"
        activeOptions={{ exact: true }}
        activeProps={{ 'aria-current': 'page' }}
      >
        <House />
        <span>Home</span>
      </Link>
      <Link
        to="/activity"
        className="bottombar__item"
        activeProps={{ 'aria-current': 'page' }}
      >
        <Inbox />
        <span>Activity</span>
        {attention > 0 && <span className="row__badge">{attention}</span>}
      </Link>
      <Link
        to="/files"
        className="bottombar__item"
        activeProps={{ 'aria-current': 'page' }}
      >
        <Folder />
        <span>Files</span>
      </Link>
      <Link
        to="/you"
        className="bottombar__item"
        activeProps={{ 'aria-current': 'page' }}
      >
        <UserRound />
        <span>You</span>
      </Link>
    </nav>
  )
}
