import { createFileRoute, useNavigate } from '@tanstack/react-router'
import { notRetired, useAgents, useTeams } from '../app/queries'
import { Skeleton } from '../components/primitives'
import { TeamEditor } from '../features/teams/TeamEditor'

export const Route = createFileRoute('/_app/teams/new')({ component: NewTeam })

function NewTeam() {
  const teams = useTeams()
  const agents = useAgents()
  const navigate = useNavigate()
  return (
    <div className="page team-page">
      <header className="page__head">
        <h1>New team</h1>
      </header>
      <div className="page__body">
        {teams.isPending || agents.isPending ? (
          <Skeleton lines={4} width="50%" />
        ) : (
          <TeamEditor
            teams={teams.data ?? []}
            agents={notRetired(agents.data)}
            onDone={(team) =>
              void navigate({
                to: '/teams/$teamId',
                params: { teamId: team.id },
              })
            }
            onCancel={() => void navigate({ to: '/' })}
          />
        )}
      </div>
    </div>
  )
}
