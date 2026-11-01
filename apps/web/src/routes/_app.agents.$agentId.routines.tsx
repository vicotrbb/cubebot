import { createFileRoute } from '@tanstack/react-router'
import { can } from '../api/types'
import { useAgentScreen } from '../features/agent/context'
import { Routines } from '../features/routines/Routines'

export const Route = createFileRoute('/_app/agents/$agentId/routines')({
  component: RoutinesTab,
})

function RoutinesTab() {
  const { agent, session } = useAgentScreen()
  return (
    <Routines
      agent={agent}
      editable={can(agent, 'conversation_send', session)}
    />
  )
}
