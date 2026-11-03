import { Link } from '@tanstack/react-router'
import { useEffect, useState } from 'react'
import {
  api,
  failure,
  ok,
  type InstalledSkill,
  type Skill,
} from '../../api/client'
import type { AgentSummary } from '../../api/types'
import { Alert, Empty, Skeleton } from '../../components/primitives'

/** The agent's installed skills, pinned to a revision, and what the library offers. */
export function InstalledSkills({ agent }: { agent: AgentSummary }) {
  const [installed, setInstalled] = useState<InstalledSkill[]>([])
  const [library, setLibrary] = useState<Skill[]>([])
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState('')
  async function load() {
    const [mine, all] = await listSkills(agent.id)
    setInstalled(mine)
    setLibrary(all)
  }
  async function change(action: () => Promise<unknown>) {
    setError('')
    try {
      await action()
    } catch (error) {
      setError(failure(error))
    }
    await load().catch((error) => setError(failure(error)))
  }
  const install = (skill: string, revision: number) =>
    change(() =>
      ok(
        api.PUT('/api/agents/{agent}/skills/{skill}', {
          params: { path: { agent: agent.id, skill } },
          body: { revision },
        }),
      ),
    )
  useEffect(() => {
    void listSkills(agent.id)
      .then(([mine, all]) => {
        setInstalled(mine)
        setLibrary(all)
      })
      .catch((error) => setError(failure(error)))
      .finally(() => setLoading(false))
  }, [agent.id])
  const pinned = new Map(installed.map((skill) => [skill.skill, skill]))
  return (
    <>
      <section className="panel__section">
        <h2>Skills</h2>
        <p className="panel__lead">
          The agent loads an installed skill when a task needs it. A new
          revision in the library does not change an installation until you
          update it. <Link to="/skills">Open the library</Link> to write or
          revise skills.
        </p>
        <Alert>{error}</Alert>
      </section>
      <section className="panel__section">
        <h3>Installed</h3>
        {loading && <Skeleton lines={2} />}
        {!loading && installed.length === 0 && (
          <Empty title="No skills installed." />
        )}
        <ul className="list">
          {installed.map((skill) => (
            <li key={skill.skill}>
              <div className="title">
                {skill.name}
                <span className="chip">rev {skill.revision}</span>
                {skill.current_revision > skill.revision && (
                  <span className="chip chip--paint">
                    rev {skill.current_revision} available
                  </span>
                )}
              </div>
              <p className="small muted">{skill.description}</p>
              <div className="actions" style={{ marginTop: 6 }}>
                {skill.current_revision > skill.revision && (
                  <button
                    type="button"
                    className="link-btn small"
                    onClick={() =>
                      void install(skill.skill, skill.current_revision)
                    }
                  >
                    Update
                  </button>
                )}
                <Link
                  to="/skills/$skillId"
                  params={{ skillId: skill.skill }}
                  className="small"
                >
                  Open
                </Link>
                <button
                  type="button"
                  className="link-btn small"
                  onClick={() =>
                    void change(() =>
                      ok(
                        api.DELETE('/api/agents/{agent}/skills/{skill}', {
                          params: {
                            path: { agent: agent.id, skill: skill.skill },
                          },
                        }),
                      ),
                    )
                  }
                >
                  Uninstall
                </button>
              </div>
            </li>
          ))}
        </ul>
      </section>
      <section className="panel__section">
        <h3>Library</h3>
        {!loading && library.length === 0 && (
          <Empty title="The library is empty." />
        )}
        <ul className="list">
          {library
            .filter((skill) => !pinned.has(skill.id))
            .map((skill) => (
              <li key={skill.id}>
                <div className="title">
                  {skill.name}
                  <span className="chip">rev {skill.revision}</span>
                </div>
                <p className="small muted">{skill.description}</p>
                <div className="actions" style={{ marginTop: 6 }}>
                  <button
                    type="button"
                    className="link-btn small"
                    onClick={() => void install(skill.id, skill.revision)}
                  >
                    Install
                  </button>
                  <Link
                    to="/skills/$skillId"
                    params={{ skillId: skill.id }}
                    className="small"
                  >
                    Open
                  </Link>
                </div>
              </li>
            ))}
        </ul>
      </section>
    </>
  )
}

function listSkills(agent: string) {
  return Promise.all([
    ok(api.GET('/api/agents/{agent}/skills', { params: { path: { agent } } })),
    ok(api.GET('/api/skills')),
  ])
}
