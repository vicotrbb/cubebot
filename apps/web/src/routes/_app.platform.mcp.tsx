import { createFileRoute } from '@tanstack/react-router'
import { McpServers } from '../features/mcp/McpServers'
import '../features/platform/platform.css'

export const Route = createFileRoute('/_app/platform/mcp')({
  component: McpServers,
})
