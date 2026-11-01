import { createFileRoute } from '@tanstack/react-router'
import { useAgentScreen } from '../features/agent/context'
import { Dreams } from '../features/memory/Dreams'

export const Route = createFileRoute('/_app/agents/$agentId/dreams')({
  component: DreamsTab,
})

function DreamsTab() {
  return <Dreams agent={useAgentScreen().agent} />
}
