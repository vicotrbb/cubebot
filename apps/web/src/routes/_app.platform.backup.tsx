import { createFileRoute } from '@tanstack/react-router'
import { PlatformBackup } from '../features/platform/Backup'

export const Route = createFileRoute('/_app/platform/backup')({
  component: PlatformBackup,
})
