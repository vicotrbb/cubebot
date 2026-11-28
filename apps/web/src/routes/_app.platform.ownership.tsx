import { createFileRoute } from '@tanstack/react-router'
import { PlatformOwnership } from '../features/platform/Ownership'

export const Route = createFileRoute('/_app/platform/ownership')({
  component: PlatformOwnership,
})
