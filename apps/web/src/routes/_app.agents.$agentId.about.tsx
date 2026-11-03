import { createFileRoute } from '@tanstack/react-router'
import { About } from '../features/agent/About'
import { useAgentScreen } from '../features/agent/context'

export const Route = createFileRoute('/_app/agents/$agentId/about')({
  component: AboutTab,
})

function AboutTab() {
  const { agent } = useAgentScreen()
  return <About key={agent.id} agent={agent} />
}
