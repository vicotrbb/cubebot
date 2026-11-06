import { useQuery, useQueryClient } from '@tanstack/react-query'
import { Link } from '@tanstack/react-router'
import { Globe, Terminal } from 'lucide-react'
import { useState } from 'react'
import { api, failure, ok } from '../../api/client'
import type { AgentSummary, Schemas } from '../../api/types'
import { Alert, Empty, Skeleton, Switch } from '../../components/primitives'
import { Mark } from '../connections/Mark'
import '../connections/connections.css'
import { MCP_KEY } from './McpServers'
import './mcp.css'

type AgentMcpServer = Schemas['AgentMcpServer']
type AgentConnection = Schemas['AgentConnection']

const LEVEL = { read: 'Read', write: 'Read and write', full: 'Full' } as const

export function Connections({
  agent,
  administrator,
}: {
  agent: AgentSummary
  administrator: boolean
}) {
  return (
    <div className="connections">
      <AccountsSection agent={agent} administrator={administrator} />
      <McpSection agent={agent} administrator={administrator} />
    </div>
  )
}

function AccountsSection({
  agent,
  administrator,
}: {
  agent: AgentSummary
  administrator: boolean
}) {
  const client = useQueryClient()
  const key = ['agent-connections', agent.id]
  const connections = useQuery({
    queryKey: key,
    queryFn: () =>
      ok(
        api.GET('/api/agents/{id}/connections', {
          params: { path: { id: agent.id } },
        }),
      ),
  })
  const [error, setError] = useState('')
  async function grant(connection: AgentConnection, enabled: boolean) {
    setError('')
    client.setQueryData<AgentConnection[]>(key, (current) =>
      current?.map((item) =>
        item.id === connection.id ? { ...item, enabled } : item,
      ),
    )
    try {
      await ok(
        api.PUT('/api/agents/{id}/connections/{connection}', {
          params: { path: { id: agent.id, connection: connection.id } },
          body: { enabled },
        }),
      )
    } catch (error) {
      setError(failure(error))
    }
    await client.invalidateQueries({ queryKey: key })
    await client.invalidateQueries({ queryKey: ['connections'] })
  }
  return (
    <section className="panel__section">
      <h2>Connected accounts</h2>
      <p className="panel__lead">
        {agent.name} acts through the accounts switched on, within each one’s
        permission level, and finds their tools when it needs them.{' '}
        {administrator && (
          <Link to="/platform/connections">Connect accounts in Platform</Link>
        )}
      </p>
      {connections.isPending && <Skeleton lines={2} />}
      <Alert>{connections.error ? failure(connections.error) : error}</Alert>
      {connections.data?.length === 0 && (
        <Empty title="No connected accounts yet.">
          {administrator
            ? 'Connect a mailbox, calendar, repository or chat under Platform › Connections, and every agent can use it.'
            : 'An administrator connects them for the whole installation.'}
        </Empty>
      )}
      {Boolean(connections.data?.length) && (
        <ul className="entries" aria-label="Connected accounts">
          {connections.data?.map((connection) => (
            <li className="entry" key={connection.id}>
              <Mark id={connection.connector} />
              <div className="entry__main">
                <span className="entry__name">{connection.slug}</span>
                <span className="entry__detail">{accountNote(connection)}</span>
              </div>
              <Switch
                checked={connection.enabled}
                disabled={!administrator || connection.all_agents}
                label={`${agent.name} may use ${connection.slug}`}
                onChange={(enabled) => void grant(connection, enabled)}
              />
            </li>
          ))}
        </ul>
      )}
    </section>
  )
}

function accountNote(connection: AgentConnection) {
  const problem =
    connection.status === 'sign_in_again'
      ? 'needs sign-in again'
      : connection.status === 'error'
        ? 'not working'
        : null
  const reach = connection.all_agents
    ? 'every agent'
    : connection.enabled
      ? 'granted'
      : 'not granted'
  return [connection.account, LEVEL[connection.level], problem ?? reach]
    .filter(Boolean)
    .join(' · ')
}

function McpSection({
  agent,
  administrator,
}: {
  agent: AgentSummary
  administrator: boolean
}) {
  const client = useQueryClient()
  const key = ['agent-mcp-servers', agent.id]
  const servers = useQuery({
    queryKey: key,
    queryFn: () =>
      ok(
        api.GET('/api/agents/{id}/mcp-servers', {
          params: { path: { id: agent.id } },
        }),
      ),
  })
  const [error, setError] = useState('')
  async function grant(server: AgentMcpServer, enabled: boolean) {
    setError('')
    client.setQueryData<AgentMcpServer[]>(key, (current) =>
      current?.map((item) =>
        item.id === server.id ? { ...item, enabled } : item,
      ),
    )
    try {
      await ok(
        api.PUT('/api/agents/{id}/mcp-servers/{server}', {
          params: { path: { id: agent.id, server: server.id } },
          body: { enabled },
        }),
      )
    } catch (error) {
      setError(failure(error))
    }
    await client.invalidateQueries({ queryKey: key })
    await client.invalidateQueries({ queryKey: MCP_KEY })
  }
  return (
    <section className="panel__section">
      <h2>MCP servers</h2>
      <p className="panel__lead">
        {agent.name} can call the tools of the servers switched on, from its
        next turn.{' '}
        {administrator && (
          <Link to="/platform/mcp">Add or edit servers in Platform</Link>
        )}
      </p>
      {servers.isPending && <Skeleton lines={2} />}
      <Alert>{servers.error ? failure(servers.error) : error}</Alert>
      {servers.data?.length === 0 && (
        <Empty title="No MCP servers in this installation.">
          {administrator
            ? 'Add one under Platform › MCP servers and every agent can use it.'
            : 'An administrator adds them for the whole installation.'}
        </Empty>
      )}
      {Boolean(servers.data?.length) && (
        <ul className="entries" aria-label="MCP servers">
          {servers.data?.map((server) => (
            <li className="entry" key={server.id}>
              <span className="entry__icon" aria-hidden="true">
                {server.transport === 'stdio' ? <Terminal /> : <Globe />}
              </span>
              <div className="entry__main">
                <span className="entry__name">{server.name}</span>
                <span className="entry__detail">{note(server)}</span>
              </div>
              <Switch
                checked={server.enabled}
                disabled={!administrator || server.all_agents}
                label={`${agent.name} may use ${server.name}`}
                onChange={(enabled) => void grant(server, enabled)}
              />
            </li>
          ))}
        </ul>
      )}
    </section>
  )
}

function note(server: AgentMcpServer) {
  if (server.all_agents) return 'Shared with every agent'
  return server.enabled ? 'Granted to this agent' : 'Not granted'
}
