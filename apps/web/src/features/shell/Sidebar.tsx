import { Link, useLocation, useNavigate } from '@tanstack/react-router'
import {
  BookOpen,
  ChevronDown,
  Folder,
  Hash,
  Inbox,
  Library,
  ListChecks,
  LogOut,
  Megaphone,
  Moon,
  Plus,
  Radar,
  Search,
  Settings,
  Star,
  Sun,
  SunMoon,
  UserRound,
} from 'lucide-react'
import { type ReactNode } from 'react'
import type { AgentSummary, RoomSummary } from '../../api/types'
import {
  signOut,
  unread,
  useUnreadActivity,
  useAgents,
  useBroadcasts,
  useInstallation,
  usePreferences,
  useRooms,
  useSavePreferences,
  useSession,
  useTasks,
} from '../../app/queries'
import { applyTheme, type Theme } from '../../app/theme'
import { agentHover, taskHover } from '../../components/agentHover'
import { Cube, moodOf } from '../../components/Cube'
import { PersonAvatar } from '../../components/PersonAvatar'
import { Menu } from '../../components/primitives'
import { doingOf } from '../agent/doing'
import { isRunning, pileOf, titleOf } from '../tasks/status'
import { TaskMark } from '../tasks/TaskMark'
import '../tasks/tasks.css'

const TASK_TREE = 'task-tree'

export const ACTIVITY: Record<string, string> = {
  working: 'Working',
  dreaming: 'Dreaming',
  idle: 'Idle',
  sleeping: 'Sleeping',
  stopped: 'Stopped',
  retired: 'Retired',
}

export const PLATFORM = [
  ['/platform/people', 'People'],
  ['/platform/teams', 'Teams and access'],
  ['/platform/sign-in', 'Sign-in'],
  ['/platform/providers', 'AI providers'],
  ['/platform/speech', 'Speech'],
  ['/platform/tasks', 'Tasks'],
  ['/platform/connections', 'Connections'],
  ['/platform/mcp', 'MCP servers'],
  ['/platform/vault', 'Vault'],
  ['/platform/audit', 'Audit log'],
  ['/platform/health', 'Health'],
] as const

export function Sidebar({
  page = false,
  onSearch,
}: {
  page?: boolean
  onSearch?: () => void
}) {
  const session = useSession()
  const installation = useInstallation()
  const administrator = Boolean(session.data?.administrator)
  const attention = useUnreadActivity()
  return (
    <aside
      className={page ? 'sidebar sidebar--page' : 'sidebar'}
      aria-label="Sidebar"
    >
      {page && <h1 className="sr-only">Home</h1>}
      <header className="sidebar__head">
        <span className="sidebar__brand">
          <Cube size={20} />
          <span className="sidebar__name">
            {installation.data?.name ?? 'Cubebot'}
          </span>
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
        <TasksRow />
        {administrator && <BroadcastRow />}
        <Link
          to="/situation"
          className="row row--page"
          activeProps={{ 'aria-current': 'page' }}
        >
          <Radar className="row__icon" />
          <span className="row__label">Situation room</span>
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
          <Link
            to="/kb"
            className="row row--page"
            activeProps={{ 'aria-current': 'page' }}
          >
            <Library className="row__icon" />
            <span className="row__label">Knowledge base</span>
          </Link>
        </Section>
      </nav>
      <footer className="sidebar__foot">
        <AccountMenu />
        {administrator && <PlatformGear />}
      </footer>
    </aside>
  )
}

function TasksRow() {
  const tasks = useTasks()
  const preferences = usePreferences()
  const save = useSavePreferences()
  const running = tasks.data?.filter(isRunning) ?? []
  const waiting = running.filter((task) => pileOf(task) === 'needs_you').length
  const collapsed = preferences.data?.collapsed.includes(TASK_TREE) ?? false
  return (
    <>
      <div className="row-tree">
        <Link
          to="/tasks"
          className="row row--page"
          activeOptions={{ exact: true }}
          activeProps={{ 'aria-current': 'page' }}
        >
          <ListChecks className="row__icon" />
          <span className="row__label">Tasks</span>
          {waiting > 0 && (
            <span className="row__badge" title="Tasks that need you">
              {waiting}
            </span>
          )}
        </Link>
        {running.length > 0 && (
          <button
            type="button"
            className="row-tree__toggle"
            aria-expanded={!collapsed}
            aria-controls="task-tree"
            aria-label={`${collapsed ? 'Show' : 'Hide'} ${running.length} running ${running.length === 1 ? 'task' : 'tasks'}`}
            onClick={() => {
              const current = preferences.data
              if (!current) return
              save.mutate({
                ...current,
                collapsed: collapsed
                  ? current.collapsed.filter((item) => item !== TASK_TREE)
                  : [...current.collapsed, TASK_TREE],
              })
            }}
          >
            <ChevronDown />
          </button>
        )}
      </div>
      {running.length > 0 && !collapsed && (
        <ul id="task-tree" className="task-tree">
          {running.map((task) => (
            <li key={task.id}>
              <Link
                to="/tasks/$taskId"
                params={{ taskId: task.id }}
                className="row row--page"
                activeProps={{ 'aria-current': 'page' }}
                {...taskHover(task.id)}
              >
                <TaskMark task={task} />
                <span className="row__label">{titleOf(task)}</span>
              </Link>
            </li>
          ))}
        </ul>
      )}
    </>
  )
}

function BroadcastRow() {
  const feed = useBroadcasts(true)
  const last = feed.data?.messages.at(-1)?.sequence ?? 0
  const fresh = last > (feed.data?.read_sequence ?? 0)
  return (
    <Link
      to="/broadcast"
      className={fresh ? 'row row--page unread' : 'row row--page'}
      activeProps={{ 'aria-current': 'page' }}
    >
      <Megaphone className="row__icon" />
      <span className="row__label">Broadcast</span>
      {fresh && <span className="row__dot" aria-label="New broadcasts" />}
    </Link>
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

function AgentRow({ agent }: { agent: AgentSummary }) {
  const mood = moodOf(agent)
  const fresh = unread(agent)
  return (
    <Link
      to="/agents/$agentId"
      params={{ agentId: agent.id }}
      className={fresh ? 'row row--agent unread' : 'row row--agent'}
      activeProps={{ 'aria-current': 'page' }}
      {...agentHover(agent.id)}
    >
      <Cube avatar={agent.avatar} mood={mood} size={18} />
      <span className="row__text">
        <span className="row__label">{agent.name}</span>
        <span className="row__doing">{doingOf(agent)}</span>
      </span>
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
    <Menu
      className="menu--right"
      trigger={<Plus />}
      triggerClassName="btn btn--ghost btn--icon btn--sm"
      label="New"
    >
      <Link to="/agents/new" className="menu__item">
        New agent
      </Link>
      <Link to="/rooms/new" className="menu__item">
        New room
      </Link>
    </Menu>
  )
}

function PlatformGear() {
  const inPlatform = useLocation({
    select: (location) => location.pathname.startsWith('/platform'),
  })
  return (
    <Link
      to={PLATFORM[0][0]}
      className="btn btn--ghost btn--icon sidebar__gear"
      aria-label="Platform settings"
      title="Platform settings"
      aria-current={inPlatform ? 'page' : undefined}
    >
      <Settings />
    </Link>
  )
}

function AccountMenu() {
  const session = useSession()
  const preferences = usePreferences()
  const save = useSavePreferences()
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
    <Menu
      className="menu--up sidebar__account"
      triggerClassName="row row--account"
      trigger={
        <>
          <PersonAvatar account={session.data?.id} name={name} />
          <span className="account__who">
            <span className="row__label">{name}</span>
            <span className="faint">
              {session.data?.administrator ? 'Administrator' : 'Member'}
            </span>
          </span>
        </>
      }
    >
      <Link to="/you" className="menu__item">
        <UserRound /> Profile
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
        <ThemeIcon /> Theme: {THEMES[theme]}
      </button>
      <div className="menu__sep" role="separator" />
      <button
        type="button"
        className="menu__item"
        onClick={async () => {
          await signOut()
          await navigate({ to: '/sign-in' })
        }}
      >
        <LogOut /> Sign out
      </button>
    </Menu>
  )
}

const THEMES: Record<Theme, string> = {
  system: 'Match system',
  light: 'Light',
  dark: 'Dark',
}

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
