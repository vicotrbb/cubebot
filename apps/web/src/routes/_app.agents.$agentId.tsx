import {
  createFileRoute,
  Link,
  Outlet,
  useChildMatches,
  useNavigate,
} from '@tanstack/react-router'
import { ChevronLeft, Monitor, PanelRight, X } from 'lucide-react'
import {
  can,
  type AgentSummary,
  type Capability,
  type Session,
} from '../api/types'
import { useAgent, useSession } from '../app/queries'
import { Cube, moodOf } from '../components/Cube'
import { Empty, Skeleton } from '../components/primitives'
import { AgentProvider } from '../features/agent/context'
import { Conversation } from '../features/chat/Conversation'
import { useConversation } from '../features/chat/useConversation'
import { LoginBanner } from '../features/desktop/LoginBanner'
import { ACTIVITY, FavoriteButton } from '../features/shell/Sidebar'
import { useIsPhone } from '../features/shell/viewport'
import '../features/agent/agent.css'

export const Route = createFileRoute('/_app/agents/$agentId')({
  component: AgentScreen,
})

/** The panel tabs, each a child route; the viewer sees the ones their grants allow. */
const TABS: { to: string; label: string; needs: Capability }[] = [
  { to: '/agents/$agentId/about', label: 'About', needs: 'agent_manage' },
  { to: '/agents/$agentId/memories', label: 'Memories', needs: 'memory_read' },
  { to: '/agents/$agentId/dreams', label: 'Dreams', needs: 'memory_read' },
  { to: '/agents/$agentId/skills', label: 'Skills', needs: 'agent_manage' },
  {
    to: '/agents/$agentId/routines',
    label: 'Routines',
    needs: 'conversation_read',
  },
  { to: '/agents/$agentId/vault', label: 'Vault', needs: 'browser_view' },
  { to: '/agents/$agentId/files', label: 'Files', needs: 'workspace_read' },
  { to: '/agents/$agentId/access', label: 'Access', needs: 'agent_manage' },
  { to: '/agents/$agentId/status', label: 'Status', needs: 'agent_manage' },
]

function AgentScreen() {
  const { agentId } = Route.useParams()
  // Remount per agent so the conversation state starts fresh instead of resetting in effects.
  return <AgentScreenFor key={agentId} agentId={agentId} />
}

function AgentScreenFor({ agentId }: { agentId: string }) {
  const session = useSession()
  const { agent, isPending } = useAgent(agentId)
  const phone = useIsPhone()
  const navigate = useNavigate()
  const children = useChildMatches()
  const panelOpen = children.length > 0
  const conversation = useConversation(agentId)
  if (isPending || !session.data)
    return (
      <div className="page__body">
        <Skeleton lines={3} width="40%" />
      </div>
    )
  if (!agent)
    return (
      <Empty title="This agent is unavailable.">
        It may have been retired, or you no longer have access to it.
      </Empty>
    )
  const me = session.data
  const tabs = TABS.filter((tab) => can(agent, tab.needs, me))
  const firstTab = tabs[0]
  const readable = can(agent, 'conversation_read', me) || me.administrator
  return (
    <AgentProvider
      value={{
        agent,
        session: me,
        turns: conversation.turns,
        streaming: conversation.streaming,
        addTurn: conversation.add,
      }}
    >
      <div className="agent-screen" data-panel={panelOpen}>
        <AgentHeader
          agent={agent}
          me={me}
          phone={phone}
          firstTab={firstTab}
          panelOpen={panelOpen}
        />
        {can(agent, 'browser_view', me) && (
          <LoginBanner
            agent={agent}
            onTake={() =>
              void navigate({
                to: '/agents/$agentId/desktop',
                params: { agentId: agent.id },
                search: { take: true },
              })
            }
          />
        )}
        <div className="agent-split">
          {readable ? (
            <Conversation
              agent={agent}
              accountId={me.id}
              you={me.login}
              canSend={can(agent, 'conversation_send', me)}
              turns={conversation.turns}
              streaming={conversation.streaming}
              loaded={conversation.loaded}
              readError={conversation.readError}
              add={conversation.add}
              onOpenThread={(turn) =>
                void navigate({
                  to: '/agents/$agentId/thread/$turnId',
                  params: { agentId: agent.id, turnId: turn.id },
                })
              }
            />
          ) : (
            <Empty title="You can see this agent, not its conversation.">
              Ask an administrator for conversation access if you need to read
              or send messages.
            </Empty>
          )}
          {panelOpen && (
            <aside className="panel" aria-label="Details">
              <div className="panel__head">
                <nav className="tabs panel__tabs" aria-label="Agent details">
                  {tabs.map((tab) => (
                    <Link
                      key={tab.to}
                      to={tab.to}
                      params={{ agentId: agent.id }}
                      activeProps={{ 'aria-current': 'page' }}
                    >
                      {tab.label}
                    </Link>
                  ))}
                </nav>
                <Link
                  to="/agents/$agentId"
                  params={{ agentId: agent.id }}
                  className="btn btn--ghost btn--icon btn--sm"
                  aria-label="Back to conversation"
                >
                  <X />
                </Link>
              </div>
              <div className="panel__body">
                <Outlet />
              </div>
            </aside>
          )}
        </div>
      </div>
    </AgentProvider>
  )
}

function AgentHeader({
  agent,
  me,
  phone,
  firstTab,
  panelOpen,
}: {
  agent: AgentSummary
  me: Session
  phone: boolean
  firstTab: (typeof TABS)[number] | undefined
  panelOpen: boolean
}) {
  const mood = moodOf(agent)
  return (
    <header className="agent-head">
      {phone && (
        <Link
          to="/"
          className="btn btn--ghost btn--icon"
          aria-label="Back to home"
        >
          <ChevronLeft />
        </Link>
      )}
      <Cube avatar={agent.avatar} mood={mood} size={36} />
      <div className="agent-head__title">
        <h1>{agent.name}</h1>
        <p className="faint small">
          <span className={`dot dot--${agent.activity}`} />{' '}
          {ACTIVITY[agent.activity] ?? agent.activity}
          {agent.desktop_active && ' · using its desktop'}
        </p>
      </div>
      <div className="agent-head__actions">
        <FavoriteButton kind="agent" id={agent.id} />
        {can(agent, 'browser_view', me) && (
          <Link
            to="/agents/$agentId/desktop"
            params={{ agentId: agent.id }}
            className="btn btn--sm"
            aria-label="Desktop"
          >
            <Monitor />
            <span className="agent-head__word">Desktop</span>
            {agent.desktop_active && (
              <span className="dot dot--working" aria-label="live" />
            )}
          </Link>
        )}
        {firstTab && !panelOpen && (
          <Link
            to={firstTab.to}
            params={{ agentId: agent.id }}
            className="btn btn--sm"
            aria-label="Details"
          >
            <PanelRight />
            <span className="agent-head__word">Details</span>
          </Link>
        )}
      </div>
    </header>
  )
}
