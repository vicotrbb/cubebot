import { Link } from '@tanstack/react-router'
import { LayoutGrid, Plus } from 'lucide-react'
import { useState } from 'react'
import { logos } from '../../../../../modules/brand/logos'
import type { AgentSummary } from '../../api/types'
import { useAgents, useSession } from '../../app/queries'
import { Cube, moodOf } from '../../components/Cube'
import { Alert } from '../../components/primitives'
import { bytes } from '../vitals/reading'
import { useApps, useConnectors, type App, type AppState } from './api'
import { NewApp } from './NewApp'
import './apps.css'

export const STATE: Record<AppState, { label: string; dot: string }> = {
  running: { label: 'Running', dot: 'dot--idle' },
  starting: { label: 'Starting', dot: 'dot--working' },
  updating: { label: 'Updating', dot: 'dot--working' },
  stopped: { label: 'Stopped', dot: '' },
  failed: { label: 'Failed', dot: 'dot--failed' },
  unavailable: { label: 'Unavailable', dot: 'dot--unscheduled' },
}

export function AgentCube({
  agent,
  size,
}: {
  agent: AgentSummary | undefined
  size: number
}) {
  return (
    <Cube
      avatar={agent?.avatar ?? null}
      mood={agent ? moodOf(agent) : 'idle'}
      size={size}
    />
  )
}

export function AppIcon({ app, size = 'md' }: { app: App; size?: string }) {
  return (
    <span className={`app-icon app-icon--${size}`} aria-hidden="true">
      {app.icon || <LayoutGrid />}
    </span>
  )
}

export function AppStanding({ app }: { app: App }) {
  const state = STATE[app.state]
  return (
    <span className="app-standing" data-state={app.state}>
      <span className={`dot ${state.dot}`} aria-hidden="true" />
      <span>{state.label}</span>
      {app.state === 'running' && app.memory != null && (
        <span className="app-standing__memory" title="Memory in use">
          {bytes(app.memory)}
        </span>
      )}
    </span>
  )
}

function Marks({
  app,
  connectorOf,
}: {
  app: App
  connectorOf: (slug: string) => string
}) {
  if (app.connections.length === 0) return null
  return (
    <span className="app-marks">
      {app.connections.map((slug) => {
        const logo = logos[connectorOf(slug)]
        return logo ? (
          <span key={slug} className="app-mark" title={slug}>
            <svg
              viewBox={logo.viewBox}
              role="img"
              aria-label={slug}
              dangerouslySetInnerHTML={{ __html: logo.svg }}
            />
          </span>
        ) : (
          <span key={slug} className="chip">
            {slug}
          </span>
        )
      })}
    </span>
  )
}

export function AppCard({
  app,
  agent,
  connectorOf,
}: {
  app: App
  agent: AgentSummary | undefined
  connectorOf: (slug: string) => string
}) {
  return (
    <Link
      to="/apps/$slug"
      params={{ slug: app.slug }}
      className="app-card"
      data-state={app.state}
    >
      <span className="app-card__head">
        <AppIcon app={app} />
        <span className="app-card__name">{app.name}</span>
        <Marks app={app} connectorOf={connectorOf} />
      </span>
      <span className="app-card__description">{app.description}</span>
      <span className="app-card__foot">
        <span className="app-card__agent">
          <AgentCube agent={agent} size={16} />
          <span>{app.agent_name}</span>
        </span>
        <AppStanding app={app} />
      </span>
    </Link>
  )
}

export function AppGrid({ apps }: { apps: App[] }) {
  const agents = useAgents()
  const session = useSession()
  const connectorOf = useConnectors(Boolean(session.data?.administrator))
  return (
    <ul className="apps__grid">
      {apps.map((app) => (
        <li key={app.id}>
          <AppCard
            app={app}
            agent={agents.data?.find((agent) => agent.id === app.agent_id)}
            connectorOf={connectorOf}
          />
        </li>
      ))}
    </ul>
  )
}

export function GridSkeleton({ count = 3 }: { count?: number }) {
  return (
    <ul className="apps__grid" aria-hidden="true">
      {Array.from({ length: count }, (_, index) => (
        <li key={index}>
          <span className="skeleton app-card--ghost" />
        </li>
      ))}
    </ul>
  )
}

export function Apps() {
  const [creating, setCreating] = useState(false)
  const [agent, setAgent] = useState<string | null>(null)
  return (
    <div className="page">
      <header className="page__head apps-head">
        <h1>Apps</h1>
        <button
          type="button"
          className="btn btn--primary btn--sm"
          onClick={() => setCreating(true)}
        >
          <Plus aria-hidden="true" /> New app
        </button>
      </header>
      <div className="page__body">
        <Catalog
          agent={agent}
          onAgent={setAgent}
          onNew={() => setCreating(true)}
        />
      </div>
      <NewApp
        open={creating}
        agentId={agent}
        onClose={() => setCreating(false)}
      />
    </div>
  )
}

function Catalog({
  agent,
  onAgent,
  onNew,
}: {
  agent: string | null
  onAgent: (agent: string | null) => void
  onNew: () => void
}) {
  const apps = useApps()
  if (apps.isPending)
    return (
      <div className="apps" aria-busy="true">
        <GridSkeleton />
      </div>
    )
  if (apps.isError)
    return (
      <div className="apps">
        <Alert>The apps could not be read. Retry shortly.</Alert>
      </div>
    )
  if (apps.data.length === 0) return <NoApps onNew={onNew} />
  const shown = apps.data.filter((app) => !agent || app.agent_id === agent)
  return (
    <div className="apps">
      <Filters apps={apps.data} agent={agent} onAgent={onAgent} />
      <AppGrid apps={shown.length > 0 ? shown : apps.data} />
    </div>
  )
}

function owners(apps: App[]) {
  const seen = new Map<string, string>()
  for (const app of apps) seen.set(app.agent_id, app.agent_name)
  return [...seen]
}

function Filters({
  apps,
  agent,
  onAgent,
}: {
  apps: App[]
  agent: string | null
  onAgent: (agent: string | null) => void
}) {
  const agents = useAgents()
  const list = owners(apps)
  if (list.length < 2) return null
  return (
    <div className="tabs apps__filters" role="group" aria-label="Agents">
      <button
        type="button"
        aria-pressed={agent === null}
        onClick={() => onAgent(null)}
      >
        All
      </button>
      {list.map(([id, name]) => {
        const summary = agents.data?.find((item) => item.id === id)
        return (
          <button
            key={id}
            type="button"
            aria-pressed={agent === id}
            onClick={() => onAgent(agent === id ? null : id)}
          >
            <AgentCube agent={summary} size={14} />
            {name}
          </button>
        )
      })}
    </div>
  )
}

function NoApps({ onNew }: { onNew: () => void }) {
  return (
    <div className="empty apps__empty">
      <span className="app-icon app-icon--lg" aria-hidden="true">
        <LayoutGrid />
      </span>
      <h2>No apps yet</h2>
      <p>
        Apps are small web tools an agent builds and runs for you, like a
        dashboard for your health data.
      </p>
      <button type="button" className="btn btn--primary" onClick={onNew}>
        <Plus aria-hidden="true" /> New app
      </button>
    </div>
  )
}
