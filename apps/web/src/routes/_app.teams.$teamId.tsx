import { createFileRoute } from '@tanstack/react-router'
import { TeamPage } from '../features/teams/TeamPage'

export const Route = createFileRoute('/_app/teams/$teamId')({
  component: TeamScreen,
})

function TeamScreen() {
  const { teamId } = Route.useParams()
  return <TeamPage key={teamId} teamId={teamId} />
}
