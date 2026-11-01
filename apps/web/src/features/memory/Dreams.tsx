import { RefreshCw } from 'lucide-react'
import { useEffect, useState } from 'react'
import { api, failure, ok, type DreamRun } from '../../api/client'
import type { AgentSummary } from '../../api/types'
import { Alert, Empty, Skeleton, when } from '../../components/primitives'

const OUTCOMES: Record<string, string> = {
  completed: 'Completed',
  yielded: 'Yielded to new work',
  failed: 'Failed',
  no_op: 'No changes',
}

export function Dreams({ agent }: { agent: AgentSummary }) {
  const [runs, setRuns] = useState<DreamRun[]>([])
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState('')
  function load() {
    setLoading(true)
    setError('')
    void listDreams(agent.id)
      .then(setRuns)
      .catch((error) => setError(failure(error)))
      .finally(() => setLoading(false))
  }
  useEffect(() => {
    void listDreams(agent.id)
      .then(setRuns)
      .catch((error) => setError(failure(error)))
      .finally(() => setLoading(false))
  }, [agent.id])
  return (
    <>
      <section className="panel__section">
        <div className="actions" style={{ justifyContent: 'space-between' }}>
          <h2>Dreams</h2>
          <button
            type="button"
            className="btn btn--ghost btn--icon btn--sm"
            aria-label="Refresh"
            onClick={load}
          >
            <RefreshCw />
          </button>
        </div>
        <p className="panel__lead">
          While idle, the agent tidies its memories and may write down a
          procedure from recent work as a skill. Each run lists the revisions it
          actually committed.
        </p>
        <Alert>{error}</Alert>
      </section>
      <section className="panel__section">
        {loading && <Skeleton lines={3} />}
        {!loading && !error && runs.length === 0 && (
          <Empty title="No dreams yet.">
            A dream starts after the agent's idle delay with nothing left to do.
          </Empty>
        )}
        <ul className="list">
          {runs.map((run) => (
            <li key={run.id}>
              <div className="title">
                {run.outcome ? OUTCOMES[run.outcome] : 'Dreaming now'}
                <span className="faint small" style={{ fontWeight: 400 }}>
                  {when(run.started_at)} · {run.model}
                </span>
              </div>
              {run.report && (
                <p
                  className="small"
                  style={{ marginTop: 4, whiteSpace: 'pre-wrap' }}
                >
                  {run.report}
                </p>
              )}
              {run.skills.length > 0 && (
                <ul className="list" style={{ marginTop: 6 }}>
                  {run.skills.map((skill) => (
                    <li key={`${skill.skill}:${skill.revision}`}>
                      <p className="small">
                        <strong>Skill {skill.name}</strong>, revision{' '}
                        {skill.revision}
                        {skill.previous == null
                          ? ' (new, installed)'
                          : ' (installed)'}
                      </p>
                      {skill.previous != null && (
                        <p className="small faint">
                          <del>{skill.previous}</del>
                        </p>
                      )}
                      <pre className="small">{skill.content}</pre>
                    </li>
                  ))}
                </ul>
              )}
              {run.revisions.length === 0 ? (
                <p className="small faint">No memory changed.</p>
              ) : (
                <ul className="list" style={{ marginTop: 6 }}>
                  {run.revisions.map((change) => (
                    <li
                      key={`${change.memory}:${change.revision}`}
                      className="small"
                    >
                      {change.previous != null && (
                        <p className="faint">
                          <del>{change.previous}</del>
                        </p>
                      )}
                      <p>
                        {change.deleted
                          ? 'Removed from search'
                          : change.content}
                      </p>
                    </li>
                  ))}
                </ul>
              )}
            </li>
          ))}
        </ul>
      </section>
    </>
  )
}

function listDreams(agent: string) {
  return ok(
    api.GET('/api/agents/{agent}/dreams', { params: { path: { agent } } }),
  )
}
