import { createFileRoute } from '@tanstack/react-router'
import { can } from '../api/types'
import { useAgentScreen } from '../features/agent/context'
import { Memories } from '../features/memory/Memories'

export const Route = createFileRoute('/_app/agents/$agentId/memories')({
  component: MemoriesTab,
})

function MemoriesTab() {
  const { agent, session } = useAgentScreen()
  return (
    <Memories agent={agent} editable={can(agent, 'memory_edit', session)} />
  )
}
