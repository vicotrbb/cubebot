import { Link, useNavigate } from '@tanstack/react-router'
import {
  ChevronLeft,
  Loader,
  Maximize2,
  MessageSquare,
  Minimize2,
  MoreHorizontal,
  Play,
  Square,
  Trash2,
  X,
} from 'lucide-react'
import { useCallback, useEffect, useRef, useState, type ReactNode } from 'react'
import { failure } from '../../api/client'
import { can, type AgentSummary, type Session } from '../../api/types'
import { useAgent, useSession } from '../../app/queries'
import {
  Alert,
  confirmAction,
  Empty,
  Menu,
  Skeleton,
} from '../../components/primitives'
import {
  openView,
  sendToAgent,
  useAppActions,
  useApps,
  type App,
  type Opened,
} from './api'
import { AgentCube, AppIcon, AppStanding } from './Apps'
import { bridge, currentTheme } from './bridge'
import './apps.css'

const ALWAYS_FULL = '(max-width: 820px), (display-mode: standalone)'

function useAlwaysFull() {
  const [always, setAlways] = useState(() => matchMedia(ALWAYS_FULL).matches)
  useEffect(() => {
    const query = matchMedia(ALWAYS_FULL)
    const update = () => setAlways(query.matches)
    query.addEventListener('change', update)
    return () => query.removeEventListener('change', update)
  }, [])
  return always
}

function useFullScreen() {
  const always = useAlwaysFull()
  const [chosen, setChosen] = useState(false)
  const full = always || chosen
  useEffect(() => {
    if (!full) return
    const root = document.documentElement
    root.dataset.appFull = ''
    return () => {
      delete root.dataset.appFull
    }
  }, [full])
  useEffect(() => {
    if (!chosen) return
    const escape = (event: KeyboardEvent) => {
      if (event.key === 'Escape' && !document.querySelector('dialog[open]'))
        setChosen(false)
    }
    window.addEventListener('keydown', escape)
    return () => window.removeEventListener('keydown', escape)
  }, [chosen])
  return { always, full, toggle: () => setChosen((on) => !on) }
}

type Screen = ReturnType<typeof useFullScreen>

export function AppView({ slug }: { slug: string }) {
  const apps = useApps()
  const session = useSession()
  const screen = useFullScreen()
  const app = apps.data?.find((item) => item.slug === slug)
  const { agent } = useAgent(app?.agent_id ?? '')
  if (apps.isPending || !session.data)
    return (
      <div className="app-view" aria-busy="true">
        <div className="app-view__head">
          <Skeleton lines={1} width="30%" />
        </div>
      </div>
    )
  if (apps.isError)
    return (
      <div className="page__body">
        <Alert>The app could not be read. Retry shortly.</Alert>
      </div>
    )
  if (!app)
    return (
      <Empty
        title="This app is unavailable."
        action={
          <Link to="/apps" className="btn">
            Back to apps
          </Link>
        }
      >
        It may have been removed, or you no longer have access to its agent.
      </Empty>
    )
  return <Opened app={app} agent={agent} me={session.data} screen={screen} />
}

function Opened({
  app,
  agent,
  me,
  screen,
}: {
  app: App
  agent: AgentSummary | undefined
  me: Session
  screen: Screen
}) {
  const [error, setError] = useState('')
  const [sent, setSent] = useState(0)
  const actions = useAppActions()
  const canSend = agent ? can(agent, 'conversation_send', me) : me.administrator
  const ask = useCallback(
    async (text: string) => {
      await sendToAgent(app.agent_id, text, app.slug)
      setSent((count) => count + 1)
    },
    [app.agent_id, app.slug],
  )
  const dismiss = useCallback(() => setSent(0), [])
  const setDesired = (desired: 'running' | 'stopped') => {
    setError('')
    actions
      .setDesired(app.slug, desired)
      .catch((caught: unknown) => setError(failure(caught)))
  }
  return (
    <div className="app-view" data-full={screen.full}>
      <Head
        app={app}
        agent={agent}
        me={me}
        screen={screen}
        canSend={canSend}
        onDesired={setDesired}
        onError={setError}
      />
      {error && (
        <div className="app-view__alert">
          <Alert>{error}</Alert>
        </div>
      )}
      <Body
        app={app}
        canSend={canSend}
        onStart={() => setDesired('running')}
        ask={ask}
      />
      {sent > 0 && (
        <Toast
          key={sent}
          agentId={app.agent_id}
          agentName={app.agent_name}
          onClose={dismiss}
        />
      )}
    </div>
  )
}

function Head({
  app,
  agent,
  me,
  screen,
  canSend,
  onDesired,
  onError,
}: {
  app: App
  agent: AgentSummary | undefined
  me: Session
  screen: Screen
  canSend: boolean
  onDesired: (desired: 'running' | 'stopped') => void
  onError: (error: string) => void
}) {
  const stopped = app.state === 'stopped'
  return (
    <header className="app-view__head">
      {screen.always && (
        <Link
          to="/apps"
          className="btn btn--ghost btn--icon"
          aria-label="Back to apps"
        >
          <ChevronLeft />
        </Link>
      )}
      <AppIcon app={app} size="sm" />
      <div className="app-view__title">
        <h1>{app.name}</h1>
        <div className="app-view__sub">
          <AgentCube agent={agent} size={14} />
          <span className="app-view__agent">{app.agent_name}</span>
          <AppStanding app={app} />
        </div>
      </div>
      <div className="app-view__actions">
        {screen.full && !screen.always && (
          <button
            type="button"
            className="btn btn--ghost btn--icon btn--sm"
            aria-label="Exit full screen"
            title="Exit full screen"
            onClick={screen.toggle}
          >
            <Minimize2 />
          </button>
        )}
        {canSend && (
          <button
            type="button"
            className="btn btn--sm"
            onClick={() => onDesired(stopped ? 'running' : 'stopped')}
          >
            {stopped ? (
              <Play aria-hidden="true" />
            ) : (
              <Square aria-hidden="true" />
            )}
            {stopped ? 'Start' : 'Stop'}
          </button>
        )}
        <More
          app={app}
          canManage={agent ? can(agent, 'agent_manage', me) : me.administrator}
          screen={screen}
          onError={onError}
        />
      </div>
    </header>
  )
}

function More({
  app,
  canManage,
  screen,
  onError,
}: {
  app: App
  canManage: boolean
  screen: Screen
  onError: (error: string) => void
}) {
  const actions = useAppActions()
  const navigate = useNavigate()
  const remove = async () => {
    document.querySelector<HTMLElement>('.app-view__actions summary')?.focus()
    const yes = await confirmAction({
      title: `Remove ${app.name}?`,
      body: `It stops and leaves the catalog for everyone. Its files stay in ${app.agent_name}'s workspace.`,
      action: 'Remove',
    })
    if (!yes) {
      document.querySelector<HTMLElement>('.app-view__actions summary')?.focus()
      return
    }
    try {
      await actions.remove(app.slug)
      await navigate({ to: '/apps' })
    } catch (caught) {
      onError(failure(caught))
    }
  }
  return (
    <Menu
      className="menu--right"
      triggerClassName="btn btn--ghost btn--icon btn--sm"
      label="App actions"
      trigger={<MoreHorizontal />}
    >
      {!screen.always && (
        <button type="button" className="menu__item" onClick={screen.toggle}>
          {screen.full ? <Minimize2 /> : <Maximize2 />}
          {screen.full ? 'Exit full screen' : 'Open full screen'}
        </button>
      )}
      <Link
        to="/agents/$agentId"
        params={{ agentId: app.agent_id }}
        className="menu__item"
      >
        <MessageSquare />
        Ask {app.agent_name}
      </Link>
      {canManage && (
        <>
          <hr className="menu__sep" />
          <button
            type="button"
            className="menu__item menu__item--danger"
            onClick={() => void remove()}
          >
            <Trash2 />
            Remove app
          </button>
        </>
      )}
    </Menu>
  )
}

function Body({
  app,
  canSend,
  onStart,
  ask,
}: {
  app: App
  canSend: boolean
  onStart: () => void
  ask: (text: string) => Promise<void>
}) {
  if (app.state === 'running') return <Stage app={app} ask={ask} />
  if (app.state === 'stopped')
    return (
      <Notice
        app={app}
        title={`${app.name} is stopped`}
        action={
          canSend && (
            <button
              type="button"
              className="btn btn--primary"
              onClick={onStart}
            >
              <Play aria-hidden="true" /> Start
            </button>
          )
        }
      >
        Start it to open it here. It keeps running until someone stops it.
      </Notice>
    )
  return <Waiting app={app} />
}

function Waiting({ app }: { app: App }) {
  const agent = app.agent_name
  if (app.state === 'failed')
    return (
      <Notice
        app={app}
        title={`${app.name} keeps stopping`}
        action={<AskLink app={app} />}
      >
        {agent} can read its logs and fix it. Tell {agent} what you saw.
      </Notice>
    )
  if (app.state === 'unavailable') return <Unavailable app={app} />
  if (app.state === 'updating')
    return (
      <Notice app={app} title={`${agent} is updating ${app.name}`} busy>
        It opens here again as soon as the changes are done.
      </Notice>
    )
  return (
    <Notice app={app} title={`Starting ${app.name}`} busy>
      It opens here as soon as it is ready.
    </Notice>
  )
}

function Unavailable({ app, retry }: { app: App; retry?: () => void }) {
  return (
    <Notice
      app={app}
      title={`${app.name} is unavailable right now`}
      action={
        retry && (
          <button type="button" className="btn" onClick={retry}>
            Try again
          </button>
        )
      }
    >
      {app.agent_name}&apos;s sandbox cannot be reached. The app opens again
      once it is back.
    </Notice>
  )
}

function AskLink({ app }: { app: App }) {
  return (
    <Link
      to="/agents/$agentId"
      params={{ agentId: app.agent_id }}
      className="btn btn--primary"
    >
      <MessageSquare aria-hidden="true" /> Ask {app.agent_name}
    </Link>
  )
}

function Notice({
  app,
  title,
  busy = false,
  action,
  children,
}: {
  app: App
  title: string
  busy?: boolean
  action?: ReactNode
  children: ReactNode
}) {
  return (
    <div className="app-view__stage">
      <div className="empty app-state" role="status">
        <span className="app-state__icon">
          <AppIcon app={app} size="lg" />
          {busy && (
            <Loader className="spin app-state__busy" aria-hidden="true" />
          )}
        </span>
        <h2>{title}</h2>
        <p>{children}</p>
        {action}
      </div>
    </div>
  )
}

function Stage({
  app,
  ask,
}: {
  app: App
  ask: (text: string) => Promise<void>
}) {
  const [attempt, setAttempt] = useState(0)
  return (
    <Framed
      key={attempt}
      app={app}
      ask={ask}
      retry={() => setAttempt((at) => at + 1)}
    />
  )
}

function Framed({
  app,
  ask,
  retry,
}: {
  app: App
  ask: (text: string) => Promise<void>
  retry: () => void
}) {
  const [opened, setOpened] = useState<Opened | null>(null)
  const [loaded, setLoaded] = useState(false)
  const frame = useRef<HTMLIFrameElement>(null)
  useEffect(() => {
    let live = true
    void openView(app.slug, currentTheme()).then((result) => {
      if (live) setOpened(result)
    })
    return () => {
      live = false
    }
  }, [app.slug])
  const src = opened && 'src' in opened ? opened.src : null
  useEffect(() => {
    const element = frame.current
    if (!element || !src) return
    return bridge(element, { ask })
  }, [src, ask])
  if (opened && 'problem' in opened)
    return opened.problem === 'unavailable' ? (
      <Unavailable app={app} retry={retry} />
    ) : (
      <Notice
        app={app}
        title={`${app.name} could not be opened`}
        action={
          <button type="button" className="btn" onClick={retry}>
            Try again
          </button>
        }
      >
        Its link expired or changed. Trying again opens a fresh one.
      </Notice>
    )
  return (
    <div className="app-view__stage" data-loaded={loaded}>
      {!loaded && (
        <p className="app-view__opening" role="status">
          <Loader className="spin" aria-hidden="true" /> Opening {app.name}
        </p>
      )}
      {src && (
        <iframe
          ref={frame}
          className="app-view__frame"
          title={app.name}
          src={src}
          sandbox="allow-scripts allow-forms allow-popups allow-downloads"
          referrerPolicy="no-referrer"
          onLoad={() => setLoaded(true)}
        />
      )}
    </div>
  )
}

function Toast({
  agentId,
  agentName,
  onClose,
}: {
  agentId: string
  agentName: string
  onClose: () => void
}) {
  useEffect(() => {
    const timer = window.setTimeout(onClose, 6000)
    return () => window.clearTimeout(timer)
  }, [onClose])
  return (
    <div className="app-toast" role="status">
      <span>Sent to {agentName}</span>
      <Link to="/agents/$agentId" params={{ agentId }}>
        Open chat
      </Link>
      <button
        type="button"
        className="btn btn--ghost btn--icon btn--sm"
        aria-label="Dismiss"
        onClick={onClose}
      >
        <X />
      </button>
    </div>
  )
}
