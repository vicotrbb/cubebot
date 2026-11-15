import { createFileRoute } from '@tanstack/react-router'
import { Meetings } from '../features/meetings/Meetings'

export const Route = createFileRoute('/_app/meetings/')({
  component: Meetings,
})
