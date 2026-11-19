import { createFileRoute } from '@tanstack/react-router'
import { useAgentScreen } from '../features/agent/context'
import { TracePanel } from '../features/chat/TracePanel'

export const Route = createFileRoute('/_app/agents/$agentId/trace/$turnId')({
  component: AgentTrace,
})

function AgentTrace() {
  const { turnId } = Route.useParams()
  const { agent } = useAgentScreen()
  return <TracePanel key={turnId} agentId={agent.id} turnId={turnId} />
}
