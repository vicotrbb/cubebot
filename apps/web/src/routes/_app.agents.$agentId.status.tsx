import { createFileRoute } from '@tanstack/react-router'
import { useAgentScreen } from '../features/agent/context'
import { StatusTab } from '../features/agent/StatusTab'

export const Route = createFileRoute('/_app/agents/$agentId/status')({
  component: StatusTabRoute,
})

function StatusTabRoute() {
  return <StatusTab agent={useAgentScreen().agent} />
}
