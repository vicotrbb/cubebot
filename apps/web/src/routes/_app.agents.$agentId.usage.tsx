import { createFileRoute } from '@tanstack/react-router'
import { useAgentScreen } from '../features/agent/context'
import { UsageTab } from '../features/home/UsageTab'

export const Route = createFileRoute('/_app/agents/$agentId/usage')({
  component: Usage,
})

function Usage() {
  const { agent, session } = useAgentScreen()
  return <UsageTab agent={agent} session={session} />
}
