import { createFileRoute } from '@tanstack/react-router'
import { PlatformMeetings } from '../features/meetings/Settings'

export const Route = createFileRoute('/_app/platform/meetings')({
  component: PlatformMeetings,
})
