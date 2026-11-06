import { createFileRoute } from '@tanstack/react-router'
import { PlatformGeneral } from '../features/platform/General'

export const Route = createFileRoute('/_app/platform/general')({
  component: PlatformGeneral,
})
