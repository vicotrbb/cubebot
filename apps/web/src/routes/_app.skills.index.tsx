import { createFileRoute, Link } from '@tanstack/react-router'
import { useQuery } from '@tanstack/react-query'
import { Plus } from 'lucide-react'
import { api, ok } from '../api/client'
import { Alert, Empty, Skeleton, when } from '../components/primitives'

export const Route = createFileRoute('/_app/skills/')({ component: Library })

/** The installation's shared skill library (docs/adr/memory.md#reusable-skills-created-by-humans-agents-and-dreaming, docs/adr/memory.md#shared-installation-skill-library-by-default and docs/adr/memory.md#shared-skill-revisions-by-any-agent-or-user). */
function Library() {
  const skills = useQuery({
    queryKey: ['skills'],
    queryFn: () => ok(api.GET('/api/skills')),
  })
  return (
    <div className="page">
      <header className="page__head">
        <h1>Skills</h1>
        <Link
          to="/skills/$skillId"
          params={{ skillId: 'new' }}
          className="btn btn--primary btn--sm"
        >
          <Plus /> New skill
        </Link>
      </header>
      <div className="page__body page__body--list">
        <p className="muted" style={{ marginBottom: 16 }}>
          Text packages any active person or agent can publish and revise.
          Installing a revision on one agent does not change the others.
        </p>
        {skills.isPending && <Skeleton lines={4} />}
        {skills.isError && <Alert>The library could not be loaded.</Alert>}
        {skills.data?.length === 0 && (
          <Empty title="The library is empty.">
            Publish the first skill, or let an agent's dream write one from
            recent work.
          </Empty>
        )}
        <ul className="list">
          {skills.data?.map((skill) => (
            <li key={skill.id}>
              <div className="title">
                <Link to="/skills/$skillId" params={{ skillId: skill.id }}>
                  {skill.name}
                </Link>
                <span className="chip">rev {skill.revision}</span>
                <span className="faint small" style={{ fontWeight: 400 }}>
                  {when(skill.updated_at)}
                </span>
              </div>
              <p className="small muted">{skill.description}</p>
            </li>
          ))}
        </ul>
      </div>
    </div>
  )
}
