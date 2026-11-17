import { createFileRoute } from '@tanstack/react-router'
import { Apps } from '../features/apps/Apps'

export const Route = createFileRoute('/_app/apps/')({
  component: Apps,
})
