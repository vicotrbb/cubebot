import { createFileRoute } from '@tanstack/react-router'
import { useAgentScreen } from '../features/agent/context'
import { Connections } from '../features/mcp/Connections'

export const Route = createFileRoute('/_app/agents/$agentId/connections')({
  component: ConnectionsTab,
})

function ConnectionsTab() {
  const { agent, session } = useAgentScreen()
  return <Connections agent={agent} administrator={session.administrator} />
}
