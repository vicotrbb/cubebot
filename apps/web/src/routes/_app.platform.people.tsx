import { createFileRoute } from '@tanstack/react-router'
import { PlatformPeople } from '../features/platform/People'

export const Route = createFileRoute('/_app/platform/people')({
  component: PlatformPeople,
})
