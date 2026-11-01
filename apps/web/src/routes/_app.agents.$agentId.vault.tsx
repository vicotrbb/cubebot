import { createFileRoute } from '@tanstack/react-router'
import { useAgentScreen } from '../features/agent/context'
import { Vault } from '../features/vault/Vault'

export const Route = createFileRoute('/_app/agents/$agentId/vault')({
  component: VaultTab,
})

function VaultTab() {
  const { agent, session } = useAgentScreen()
  return <Vault agent={agent} administrator={session.administrator} />
}
