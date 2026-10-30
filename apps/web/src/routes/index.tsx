import { createFileRoute } from '@tanstack/react-router'
import { useEffect, useState } from 'react'
import {
  agents as listAgents,
  api,
  ApiError,
  failure,
  ok,
  type Account,
  type Agent,
} from '../api/client'
import { AgentForm, ProviderForm, SignIn } from '../features/forms'
import { Chat } from '../features/chat'
import { Dreams } from '../features/dreams'
import { Memories } from '../features/memories'

export const Route = createFileRoute('/')({ component: Home })

function Home() {
  const [session, setSession] = useState<Account | null>(null)
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState('')
  async function authenticate() {
    setSession(await ok(api.GET('/api/identity/me')))
  }
  useEffect(() => {
    void ok(api.GET('/api/identity/me'))
      .then(setSession)
      .catch((error) => {
        if (!(error instanceof ApiError && error.status === 401))
          setError(failure(error))
      })
      .finally(() => setLoading(false))
  }, [])
  if (loading)
    return (
      <main className="sign-in">
        <p role="status">Opening Cubebot…</p>
      </main>
    )
  if (!session)
    return (
      <>
        {error && (
          <p className="global-error" role="alert">
            {error}
          </p>
        )}
        <SignIn onSignedIn={authenticate} />
      </>
    )
  return (
    <Workspace
      key={session.id}
      session={session}
      onSignOut={async () => {
        await ok(api.POST('/api/identity/logout'))
        setSession(null)
        setError('')
      }}
    />
  )
}

type Screen =
  | { kind: 'chat' }
  | { kind: 'memories' }
  | { kind: 'dreams' }
  | { kind: 'agent'; existing: Agent | null }
  | { kind: 'provider' }

function Workspace({
  session,
  onSignOut,
}: {
  session: Account
  onSignOut: () => Promise<void>
}) {
  const [agents, setAgents] = useState<Agent[]>([])
  const [selected, setSelected] = useState<Agent | null>(null)
  const [screen, setScreen] = useState<Screen>({ kind: 'chat' })
  const [error, setError] = useState('')
  const [loading, setLoading] = useState(true)
  async function load() {
    applyAgents(await listAgents())
  }
  function applyAgents(all: Agent[]) {
    setAgents(all)
    setSelected(
      (current) => all.find((agent) => agent.id === current?.id) ?? null,
    )
  }
  useEffect(() => {
    void listAgents()
      .then(applyAgents)
      .catch((error) => setError(failure(error)))
      .finally(() => setLoading(false))
  }, [])
  async function saved() {
    await load()
    setScreen({ kind: 'chat' })
  }
  return (
    <main className="app-shell">
      <header className="masthead">
        <a className="wordmark" href="/">
          cubebot
        </a>
        <nav aria-label="Account">
          {session.administrator && (
            <button
              className="secondary"
              onClick={() => setScreen({ kind: 'provider' })}
            >
              AI provider
            </button>
          )}
          <button
            className="secondary"
            onClick={() => {
              void onSignOut().catch((error) => setError(failure(error)))
            }}
          >
            Sign out
          </button>
        </nav>
      </header>
      <div className="workspace-grid">
        <aside className="agent-list">
          <div className="section-title">
            <h2>Agents</h2>
            {session.administrator && (
              <button
                className="secondary"
                onClick={() => setScreen({ kind: 'agent', existing: null })}
              >
                New agent
              </button>
            )}
          </div>
          {loading && <p role="status">Loading agents…</p>}
          {!loading && agents.length === 0 && (
            <p className="muted">No authorized agents yet.</p>
          )}
          <nav aria-label="Agents">
            {agents.map((agent) => (
              <button
                key={agent.id}
                className={
                  selected?.id === agent.id
                    ? 'agent-link selected'
                    : 'agent-link'
                }
                aria-current={selected?.id === agent.id ? 'true' : undefined}
                onClick={() => {
                  setSelected(agent)
                  setScreen({ kind: 'chat' })
                  setError('')
                }}
              >
                {agent.name}{' '}
                <span className="muted">
                  {activities[agent.activity] ?? agent.activity}
                </span>
              </button>
            ))}
          </nav>
          <button
            className="text-button"
            onClick={() => {
              setError('')
              void load().catch((error) => setError(failure(error)))
            }}
          >
            Refresh agents
          </button>
        </aside>
        <div className="main-panel">
          {error && <p role="alert">{error}</p>}
          {screen.kind === 'agent' && (
            <AgentForm
              key={screen.existing?.id ?? 'new'}
              existing={screen.existing}
              onSaved={saved}
              onCancel={() => setScreen({ kind: 'chat' })}
            />
          )}
          {screen.kind === 'provider' && (
            <ProviderForm onClose={() => setScreen({ kind: 'chat' })} />
          )}
          <AgentScreen
            screen={screen}
            selected={selected}
            session={session}
            setScreen={setScreen}
          />
        </div>
      </div>
    </main>
  )
}

/** The screens about the selected agent, or the empty state without one. */
function AgentScreen({
  screen,
  selected,
  session,
  setScreen,
}: {
  screen: Screen
  selected: Agent | null
  session: Account
  setScreen: (screen: Screen) => void
}) {
  return (
    <>
      {screen.kind === 'memories' && selected && (
        <Memories
          key={selected.id}
          agent={selected}
          onClose={() => setScreen({ kind: 'chat' })}
        />
      )}
      {screen.kind === 'dreams' && selected && (
        <Dreams
          key={selected.id}
          agent={selected}
          onClose={() => setScreen({ kind: 'chat' })}
        />
      )}
      {screen.kind === 'chat' &&
        (selected ? (
          <AgentChat
            key={selected.id}
            selected={selected}
            session={session}
            onEdit={() => setScreen({ kind: 'agent', existing: selected })}
            onMemories={() => setScreen({ kind: 'memories' })}
            onDreams={() => setScreen({ kind: 'dreams' })}
          />
        ) : (
          <div className="empty">
            <h1>Your agents.</h1>
            <p>Select an agent to continue, or create one to get started.</p>
          </div>
        ))}
    </>
  )
}

const activities: Record<string, string> = {
  working: 'Working',
  dreaming: 'Dreaming',
  idle: 'Idle',
  sleeping: 'Sleeping',
  retired: 'Retired',
}

function AgentChat({
  selected,
  session,
  onEdit,
  onMemories,
  onDreams,
}: {
  selected: Agent
  session: Account
  onEdit: () => void
  onMemories: () => void
  onDreams: () => void
}) {
  return (
    <>
      <header className="conversation-header">
        <div>
          <h1>{selected.name}</h1>
          <p className="muted">
            Continuous conversation ·{' '}
            {activities[selected.activity] ?? selected.activity}
          </p>
        </div>
        <div className="actions">
          <button className="secondary" onClick={onMemories}>
            Memories
          </button>
          <button className="secondary" onClick={onDreams}>
            Dreams
          </button>
          <button className="secondary" onClick={onEdit}>
            Edit agent
          </button>
        </div>
      </header>
      <Chat id={selected.id} accountId={session.id} />
    </>
  )
}
