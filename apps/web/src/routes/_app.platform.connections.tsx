import { createFileRoute } from '@tanstack/react-router'
import { ConnectionsPage } from '../features/connections/ConnectionsPage'
import '../features/platform/platform.css'

export const Route = createFileRoute('/_app/platform/connections')({
  component: ConnectionsPage,
})
