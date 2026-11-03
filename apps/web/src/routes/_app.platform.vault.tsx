import { createFileRoute } from '@tanstack/react-router'
import { InstallationVault } from '../features/vault/InstallationVault'
import '../features/platform/platform.css'

export const Route = createFileRoute('/_app/platform/vault')({
  component: InstallationVault,
})
