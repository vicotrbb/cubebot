import { createFileRoute } from '@tanstack/react-router'
import { MeetingPage } from '../features/meetings/MeetingPage'

export const Route = createFileRoute('/_app/meetings/$meetingId')({
  validateSearch: (
    search: Record<string, unknown>,
  ): { join?: boolean | undefined } => ({
    join: search.join === true || search.join === 'true' ? true : undefined,
  }),
  component: Meeting,
})

function Meeting() {
  const { meetingId } = Route.useParams()
  const { join } = Route.useSearch()
  return <MeetingPage key={meetingId} id={meetingId} join={Boolean(join)} />
}
