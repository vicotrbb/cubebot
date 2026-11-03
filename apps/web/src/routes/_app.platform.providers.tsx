import { createFileRoute } from '@tanstack/react-router'
import { Providers } from '../features/platform/Providers'

export const Route = createFileRoute('/_app/platform/providers')({
  component: Providers,
})
