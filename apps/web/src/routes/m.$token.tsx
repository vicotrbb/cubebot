import { createFileRoute } from '@tanstack/react-router'
import { PublicMeeting } from '../features/meetings/PublicMeeting'

export const Route = createFileRoute('/m/$token')({
  ssr: false,
  head: () => ({ meta: [{ title: 'Meeting · Cubebot' }] }),
  component: Meeting,
})

function Meeting() {
  const { token } = Route.useParams()
  return <PublicMeeting token={token} />
}
