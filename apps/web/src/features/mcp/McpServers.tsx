import { useQuery, useQueryClient } from '@tanstack/react-query'
import {
  Globe,
  MoreHorizontal,
  Pencil,
  Plus,
  Terminal,
  Trash2,
} from 'lucide-react'
import { useState } from 'react'
import { api, failure, ok } from '../../api/client'
import type { Schemas } from '../../api/types'
import { useAgents } from '../../app/queries'
import {
  Alert,
  confirmAction,
  Dialog,
  Empty,
  Menu,
  Skeleton,
  Switch,
} from '../../components/primitives'
import '../vault/vault.css'
import './mcp.css'
import { McpForm } from './McpForm'

export type McpServer = Schemas['McpServerSummary']
export type SaveMcpServer = Schemas['SaveMcpServer']
export const MCP_KEY = ['mcp-servers']

export function unchanged(server: McpServer): SaveMcpServer {
  return {
    name: server.name,
    transport: server.transport,
    command: server.command ?? null,
    args: server.args,
    url: server.url ?? null,
  }
}

export function McpServers() {
  const client = useQueryClient()
  const servers = useQuery({
    queryKey: MCP_KEY,
    queryFn: () => ok(api.GET('/api/mcp-servers')),
  })
  const [editing, setEditing] = useState<McpServer | 'new' | null>(null)
  const [error, setError] = useState('')
  const refresh = () => client.invalidateQueries({ queryKey: MCP_KEY })
  async function act(action: () => Promise<unknown>) {
    setError('')
    try {
      await action()
      await refresh()
    } catch (error) {
      setError(failure(error))
    }
  }
  const remove = async (server: McpServer) =>
    (await confirmAction({
      title: `Remove ${server.name}?`,
      body: 'Agents lose its tools from their next turn.',
      action: 'Remove',
    })) &&
    void act(() =>
      ok(
        api.DELETE('/api/mcp-servers/{server}', {
          params: { path: { server: server.id } },
        }),
      ),
    )
  const share = (server: McpServer, everyone: boolean) =>
    void act(() =>
      ok(
        api.PUT('/api/mcp-servers/{server}', {
          params: { path: { server: server.id } },
          body: {
            ...unchanged(server),
            access: everyone
              ? { mode: 'all_agents' }
              : { mode: 'explicit_grants', agents: [] },
          },
        }),
      ),
    )
  const add = (
    <button
      type="button"
      className="btn btn--primary"
      onClick={() => setEditing('new')}
    >
      <Plus /> Add server
    </button>
  )
  return (
    <div className="vault-page">
      <div className="vault-page__head">
        <p className="platform__lead">
          Tools from Model Context Protocol servers, offered to agents as{' '}
          <code>server__tool</code>. Each is available to every agent unless you
          limit it; an agent's Connections tab grants a limited one.
        </p>
        {Boolean(servers.data?.length) && add}
      </div>
      {servers.isPending && <Skeleton lines={3} />}
      <Alert>{servers.error ? failure(servers.error) : error}</Alert>
      {servers.data?.length === 0 && (
        <Empty title="No MCP servers yet." action={add}>
          Add a local program such as{' '}
          <code>npx -y @modelcontextprotocol/server-github</code> or a remote
          URL, and agents can call its tools from their next turn.
        </Empty>
      )}
      {Boolean(servers.data?.length) && (
        <ul className="entries" aria-label="MCP servers">
          {servers.data?.map((server) => (
            <Row
              key={server.id}
              server={server}
              onShare={(everyone) => share(server, everyone)}
              onEdit={() => setEditing(server)}
              onDelete={() => remove(server)}
            />
          ))}
        </ul>
      )}
      <Dialog
        open={editing !== null}
        title={
          editing && editing !== 'new'
            ? `Edit ${editing.name}`
            : 'Add an MCP server'
        }
        onClose={() => setEditing(null)}
      >
        <McpForm
          existing={editing && editing !== 'new' ? editing : undefined}
          onDone={() => {
            setEditing(null)
            void refresh()
          }}
        />
      </Dialog>
    </div>
  )
}

function Row({
  server,
  onShare,
  onEdit,
  onDelete,
}: {
  server: McpServer
  onShare: (everyone: boolean) => void
  onEdit: () => void
  onDelete: () => void
}) {
  const agents = useAgents()
  const Icon = server.transport === 'stdio' ? Terminal : Globe
  const everyone = server.access.mode === 'all_agents'
  return (
    <li className="entry">
      <span className="entry__icon" aria-hidden="true">
        <Icon />
      </span>
      <div className="entry__main">
        <span className="entry__name">{server.name}</span>
        <span className="entry__detail mcp-detail">{detail(server)}</span>
      </div>
      <div className="entry__meta">
        <span className="faint small mcp-reach">
          {reach(server, agents.data?.length ?? 0)}
        </span>
        <span className="mcp-share">
          <Switch
            checked={everyone}
            label={`${server.name} is available to every agent`}
            onChange={onShare}
          />
          <span className="small" aria-hidden="true">
            All agents
          </span>
        </span>
        <Menu
          className="menu--right"
          trigger={<MoreHorizontal />}
          triggerClassName="btn btn--ghost btn--icon btn--sm"
          label={`Actions for ${server.name}`}
        >
          <button type="button" className="menu__item" onClick={onEdit}>
            <Pencil /> Edit
          </button>
          <button
            type="button"
            className="menu__item menu__item--danger"
            onClick={onDelete}
          >
            <Trash2 /> Remove
          </button>
        </Menu>
      </div>
    </li>
  )
}

function detail(server: McpServer) {
  const line =
    server.transport === 'stdio'
      ? [server.command, ...server.args].join(' ')
      : (server.url ?? '')
  const secrets = server.secret_names.length
  if (secrets === 0) return line
  const kind = server.transport === 'stdio' ? 'variable' : 'header'
  return `${line} · ${secrets} ${kind}${secrets === 1 ? '' : 's'}`
}

function reach(server: McpServer, total: number) {
  if (server.access.mode === 'all_agents')
    return total === 1 ? '1 agent' : `${total} agents`
  const count = server.access.agents.length
  if (count === 0) return 'No agents yet'
  return count === 1 ? '1 agent' : `${count} agents`
}
