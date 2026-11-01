import { useQuery, useQueryClient } from '@tanstack/react-query'
import { createFileRoute, Link, useNavigate } from '@tanstack/react-router'
import { ChevronLeft } from 'lucide-react'
import { api, ok } from '../api/client'
import { Skeleton } from '../components/primitives'
import { SkillEditor } from '../features/skills/SkillEditor'

export const Route = createFileRoute('/_app/skills/$skillId')({
  component: SkillPage,
})

function SkillPage() {
  const { skillId } = Route.useParams()
  const navigate = useNavigate()
  const client = useQueryClient()
  const skills = useQuery({
    queryKey: ['skills'],
    queryFn: () => ok(api.GET('/api/skills')),
    enabled: skillId !== 'new',
  })
  const skill = skills.data?.find((item) => item.id === skillId) ?? null
  const done = () => {
    void client.invalidateQueries({ queryKey: ['skills'] })
    void navigate({ to: '/skills' })
  }
  return (
    <div className="page">
      <header className="page__head">
        <Link
          to="/skills"
          className="btn btn--ghost btn--icon"
          aria-label="Back to skills"
        >
          <ChevronLeft />
        </Link>
        <h1>{skillId === 'new' ? 'New skill' : (skill?.name ?? 'Skill')}</h1>
      </header>
      <div className="page__body page__body--narrow">
        {skillId !== 'new' && skills.isPending && <Skeleton lines={5} />}
        {(skillId === 'new' || skill) && (
          <SkillEditor key={skill?.id ?? 'new'} skill={skill} onDone={done} />
        )}
      </div>
    </div>
  )
}
