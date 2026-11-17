import { createFileRoute } from '@tanstack/react-router'
import { useAgentScreen } from '../features/agent/context'
import { AgentSpeechSection } from '../features/call/SpeechSettings'

export const Route = createFileRoute('/_app/agents/$agentId/speech')({
  component: SpeechTab,
})

function SpeechTab() {
  const { agent } = useAgentScreen()
  return <AgentSpeechSection key={agent.id} agent={agent} />
}
