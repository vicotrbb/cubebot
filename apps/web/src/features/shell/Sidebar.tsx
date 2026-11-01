import { useQueryClient } from '@tanstack/react-query'
import { Link, useNavigate } from '@tanstack/react-router'
import {
  BookOpen,
  ChevronDown,
  Folder,
  Hash,
  Inbox,
  LogOut,
  Moon,
  Plus,
  Search,
  Settings,
  Star,
  Sun,
  SunMoon,
  UserRound,
} from 'lucide-react'
import { type ReactNode } from 'react'
import { api, ok } from '../../api/client'
import type { AgentSummary, RoomSummary } from '../../api/types'
import {
  keys,
  unread,
  useActivity,
  useAgents,
  useInstallation,
  usePreferences,
  useRooms,
  useSavePreferences,
  useSession,
} from '../../app/queries'
import { applyTheme, type Theme } from '../../app/theme'
import { Cube, moodOf } from '../../components/Cube'

const ACTIVITY: Record<string, string> = {
  working: 'Working',
  dreaming: 'Dreaming',
  idle: 'Idle',
  sleeping: 'Sleeping',
  retired: 'Retired',
}

export const PLATFORM = [
  ['/platform/people', 'People'],
  ['/platform/teams', 'Teams and access'],
  ['/platform/sign-in', 'Sign-in'],
  ['/platform/providers', 'AI providers'],
  ['/platform/vault', 'Vault'],
  ['/platform/audit', 'Audit log'],
  ['/platform/health', 'Health'],
] as const

/** The installation's navigation; on phones it is the Home page (`page`). */
export function Sidebar({
  page = false,
  onSearch,
}: {
  page?: boolean
  onSearch?: () => void
}) {
  const session = useSession()
  const installation = useInstallation()
  const activity = useActivity()
  const administrator = Boolean(session.data?.administrator)
  const attention = activity.data?.length ?? 0
  return (
    <aside className={page ? 'sidebar sidebar--page' : 'sidebar'}>
      <header className="sidebar__head">
        <span className="sidebar__name">
          {installation.data?.name ?? 'Cubebot'}
        </span>
        {onSearch && (
          <button
            type="button"
            className="btn btn--ghost btn--icon btn--sm"
            aria-label="Search (⌘K)"
            title="Search ⌘K"
            onClick={onSearch}
          >
            <Search />
          </button>
        )}
        {administrator && <NewMenu />}
      </header>
      <nav className="sidebar__scroll" aria-label="Workspace">
        <Link
          to="/activity"
          className="row row--page"
          activeProps={{ 'aria-current': 'page' }}
        >
          <Inbox className="row__icon" />
          <span className="row__label">Activity</span>
          {attention > 0 && <span className="row__badge">{attention}</span>}
        </Link>
        <Favorites />
        <AgentsSection administrator={administrator} />
        <RoomsSection />
        <Section id="library" title="Library">
          <Link
            to="/files"
            className="row row--page"
            activeProps={{ 'aria-current': 'page' }}
          >
            <Folder className="row__icon" />
            <span className="row__label">Files</span>
          </Link>
          <Link
            to="/skills"
            className="row row--page"
            activeProps={{ 'aria-current': 'page' }}
          >
            <BookOpen className="row__icon" />
            <span className="row__label">Skills</span>
          </Link>
        </Section>
        {administrator && (
          <Section id="platform" title="Platform">
            {PLATFORM.map(([to, label]) => (
              <Link
                key={to}
                to={to}
                className="row row--page"
                activeProps={{ 'aria-current': 'page' }}
              >
                <span className="row__label">{label}</span>
              </Link>
            ))}
          </Section>
        )}
      </nav>
      <AccountMenu />
    </aside>
  )
}

function Favorites() {
  const agents = useAgents()
  const rooms = useRooms()
  const preferences = usePreferences()
  const favorites = preferences.data?.favorites ?? []
  const favoriteAgents = favorites
    .filter((item) => item.kind === 'agent')
    .map((item) => agents.data?.find((agent) => agent.id === item.id))
    .filter((agent): agent is AgentSummary => Boolean(agent))
  const favoriteRooms = favorites
    .filter((item) => item.kind === 'room')
    .map((item) => rooms.data?.find((room) => room.id === item.id))
    .filter((room): room is RoomSummary => Boolean(room))
  if (favoriteAgents.length === 0 && favoriteRooms.length === 0) return null
  return (
    <Section id="favorites" title="Favorites">
      {favoriteAgents.map((agent) => (
        <AgentRow key={agent.id} agent={agent} />
      ))}
      {favoriteRooms.map((room) => (
        <RoomRow key={room.id} room={room} />
      ))}
    </Section>
  )
}

function AgentsSection({ administrator }: { administrator: boolean }) {
  const agents = useAgents()
  return (
    <Section id="agents" title="Agents" label="Agents">
      {agents.isPending && <RowSkeleton />}
      {agents.data?.length === 0 && (
        <p className="row row--note faint">
          {administrator ? 'No agents yet.' : 'No agents shared with you.'}
        </p>
      )}
      {agents.data?.map((agent) => (
        <AgentRow key={agent.id} agent={agent} />
      ))}
    </Section>
  )
}

function RoomsSection() {
  const rooms = useRooms()
  return (
    <Section id="rooms" title="Rooms" label="Rooms">
      {rooms.data?.length === 0 && (
        <p className="row row--note faint">No rooms yet.</p>
      )}
      {rooms.data?.map((room) => (
        <RoomRow key={room.id} room={room} />
      ))}
    </Section>
  )
}

function Section({
  id,
  title,
  label,
  children,
}: {
  id: string
  title: string
  label?: string
  children: ReactNode
}) {
  const preferences = usePreferences()
  const save = useSavePreferences()
  const collapsed = preferences.data?.collapsed.includes(id) ?? false
  return (
    <section className="section">
      <button
        type="button"
        className="section__head"
        aria-expanded={!collapsed}
        onClick={() => {
          const current = preferences.data
          if (!current) return
          save.mutate({
            ...current,
            collapsed: collapsed
              ? current.collapsed.filter((item) => item !== id)
              : [...current.collapsed, id],
          })
        }}
      >
        <ChevronDown
          className={collapsed ? 'section__chev collapsed' : 'section__chev'}
        />
        {title}
      </button>
      {!collapsed && (
        <div
          className="section__body"
          role="navigation"
          aria-label={label ?? title}
        >
          {children}
        </div>
      )}
    </section>
  )
}

export function AgentRow({ agent }: { agent: AgentSummary }) {
  const mood = moodOf(agent)
  const fresh = unread(agent)
  return (
    <Link
      to="/agents/$agentId"
      params={{ agentId: agent.id }}
      className={fresh ? 'row row--agent unread' : 'row row--agent'}
      activeProps={{ 'aria-current': 'page' }}
    >
      <Cube avatar={agent.avatar} mood={mood} size={22} />
      <span className="row__label">{agent.name}</span>
      {agent.pending_login && <span className="row__flag">Needs you</span>}
      <span
        className={`dot dot--${agent.activity}`}
        title={ACTIVITY[agent.activity] ?? agent.activity}
      />
    </Link>
  )
}

function RoomRow({ room }: { room: RoomSummary }) {
  const fresh = unread(room)
  return (
    <Link
      to="/rooms/$roomId"
      params={{ roomId: room.id }}
      className={fresh ? 'row row--room unread' : 'row row--room'}
      activeProps={{ 'aria-current': 'page' }}
    >
      <Hash className="row__icon" />
      <span className="row__label">{room.name}</span>
      <span className="faint small tabular">{room.members.length}</span>
    </Link>
  )
}

function RowSkeleton() {
  return (
    <div className="row row--note" aria-hidden="true">
      <span className="skeleton" style={{ width: '70%' }} />
    </div>
  )
}

function NewMenu() {
  return (
    <details className="menu menu--right">
      <summary className="btn btn--ghost btn--icon btn--sm" aria-label="New">
        <Plus />
      </summary>
      <div className="menu__list">
        <Link to="/agents/new" className="menu__item">
          New agent
        </Link>
        <Link to="/rooms/new" className="menu__item">
          New room
        </Link>
      </div>
    </details>
  )
}

function AccountMenu() {
  const session = useSession()
  const preferences = usePreferences()
  const save = useSavePreferences()
  const client = useQueryClient()
  const navigate = useNavigate()
  const theme = preferences.data?.theme ?? 'system'
  const name = session.data?.name || session.data?.login || 'You'
  const nextTheme: Record<Theme, Theme> = {
    system: 'light',
    light: 'dark',
    dark: 'system',
  }
  const ThemeIcon = theme === 'light' ? Sun : theme === 'dark' ? Moon : SunMoon
  return (
    <details className="menu menu--up sidebar__account">
      <summary className="row row--account">
        <span className="avatar-initial" aria-hidden="true">
          {name.slice(0, 1).toUpperCase()}
        </span>
        <span className="row__label">{name}</span>
        <span className="faint small">
          {session.data?.administrator ? 'Administrator' : 'Member'}
        </span>
      </summary>
      <div className="menu__list">
        <Link to="/you" className="menu__item">
          <UserRound /> You
        </Link>
        <button
          type="button"
          className="menu__item"
          onClick={() => {
            const next = nextTheme[theme]
            applyTheme(next)
            if (preferences.data)
              save.mutate({ ...preferences.data, theme: next })
          }}
        >
          <ThemeIcon /> Theme: {theme}
        </button>
        {session.data?.administrator && (
          <Link to="/platform/health" className="menu__item">
            <Settings /> Platform
          </Link>
        )}
        <button
          type="button"
          className="menu__item"
          onClick={async () => {
            await ok(api.POST('/api/identity/logout'))
            client.setQueryData(keys.session, null)
            client.clear()
            await navigate({ to: '/sign-in' })
          }}
        >
          <LogOut /> Sign out
        </button>
      </div>
    </details>
  )
}

/** Star control used by headers to pin an agent or room in Favorites. */
export function FavoriteButton({
  kind,
  id,
}: {
  kind: 'agent' | 'room'
  id: string
}) {
  const preferences = usePreferences()
  const save = useSavePreferences()
  const current = preferences.data
  const starred = current?.favorites.some(
    (item) => item.kind === kind && item.id === id,
  )
  return (
    <button
      type="button"
      className="btn btn--ghost btn--icon"
      aria-pressed={starred}
      aria-label={starred ? 'Remove from favorites' : 'Add to favorites'}
      title={starred ? 'Remove from favorites' : 'Add to favorites'}
      onClick={() => {
        if (!current) return
        save.mutate({
          ...current,
          favorites: starred
            ? current.favorites.filter(
                (item) => !(item.kind === kind && item.id === id),
              )
            : [...current.favorites, { kind, id }],
        })
      }}
    >
      <Star fill={starred ? 'currentColor' : 'none'} />
    </button>
  )
}
