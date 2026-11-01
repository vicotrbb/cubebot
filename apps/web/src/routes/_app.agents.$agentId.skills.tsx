import { createFileRoute } from '@tanstack/react-router'
import { useAgentScreen } from '../features/agent/context'
import { InstalledSkills } from '../features/skills/InstalledSkills'

export const Route = createFileRoute('/_app/agents/$agentId/skills')({
  component: SkillsTab,
})

function SkillsTab() {
  return <InstalledSkills agent={useAgentScreen().agent} />
}
