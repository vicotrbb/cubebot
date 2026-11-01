import { createFileRoute } from '@tanstack/react-router'
import { Access } from '../features/agent/Access'
import { useAgentScreen } from '../features/agent/context'

export const Route = createFileRoute('/_app/agents/$agentId/access')({
  component: AccessTab,
})

function AccessTab() {
  return <Access agent={useAgentScreen().agent} />
}
