import { createFileRoute } from '@tanstack/react-router'
import { useAgentScreen } from '../features/agent/context'
import { Files } from '../features/files/Files'

export const Route = createFileRoute('/_app/agents/$agentId/files')({
  component: FilesTab,
})

function FilesTab() {
  const { agent } = useAgentScreen()
  return (
    <>
      <section className="panel__section">
        <h2>Files</h2>
        <p className="panel__lead">
          Folders {agent.name} shared with you. Share your own from the Files
          page and the agent reads them with its file tools.
        </p>
      </section>
      <Files only={{ agent: agent.id }} />
    </>
  )
}
