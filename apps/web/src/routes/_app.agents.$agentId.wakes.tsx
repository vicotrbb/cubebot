import { createFileRoute } from '@tanstack/react-router'
import { can } from '../api/types'
import { useAgentScreen } from '../features/agent/context'
import { Wakes } from '../features/wakes/Wakes'

export const Route = createFileRoute('/_app/agents/$agentId/wakes')({
  component: WakesTab,
})

function WakesTab() {
  const { agent, session } = useAgentScreen()
  return (
    <Wakes agent={agent} editable={can(agent, 'conversation_send', session)} />
  )
}
