import { createFileRoute } from '@tanstack/react-router'
import { useAgentScreen } from '../features/agent/context'
import { AgentApps } from '../features/apps/AgentApps'

export const Route = createFileRoute('/_app/agents/$agentId/apps')({
  component: AppsTab,
})

function AppsTab() {
  const { agent } = useAgentScreen()
  return <AgentApps agent={agent} />
}
