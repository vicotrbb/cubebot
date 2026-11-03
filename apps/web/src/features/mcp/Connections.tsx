import { useQuery, useQueryClient } from '@tanstack/react-query'
import { Link } from '@tanstack/react-router'
import { Globe, Terminal } from 'lucide-react'
import { useState } from 'react'
import { api, failure, ok } from '../../api/client'
import type { AgentSummary, Schemas } from '../../api/types'
import { Alert, Empty, Skeleton, Switch } from '../../components/primitives'
import { MCP_KEY } from './McpServers'
import './mcp.css'

type AgentMcpServer = Schemas['AgentMcpServer']

/**
 * What the agent reaches beyond its computer: MCP servers, switched per agent when a server
 * is limited to chosen agents, and connected accounts, which come later.
 */
export function Connections({
  agent,
  administrator,
}: {
  agent: AgentSummary
  administrator: boolean
}) {
  return (
    <div className="connections">
      <McpSection agent={agent} administrator={administrator} />
      <section className="panel__section">
        <h2>Connections</h2>
        <Empty title="No connected accounts yet.">
          Accounts such as Slack, X or LinkedIn that an agent acts through will
          be connected here.
        </Empty>
      </section>
    </div>
  )
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
    // Shown at once; the refetch below settles it either way.
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
