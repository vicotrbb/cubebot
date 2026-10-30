import { useEffect, useState } from 'react'
import { api, failure, ok, type Agent, type DreamRun } from '../api/client'

const outcomes: Record<string, string> = {
  completed: 'Completed',
  yielded: 'Yielded to new work',
  failed: 'Failed',
  no_op: 'No changes',
}

export function Dreams({
  agent,
  onClose,
}: {
  agent: Agent
  onClose: () => void
}) {
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
    <section>
      <header className="conversation-header">
        <div>
          <h1>{agent.name}: dreams</h1>
          <p className="muted">
            While idle, the agent tidies its memories. Each run lists the
            revisions it actually committed; earlier revisions are kept.
          </p>
        </div>
        <div className="actions">
          <button className="secondary" onClick={load}>
            Refresh
          </button>
          <button className="secondary" onClick={onClose}>
            Back to conversation
          </button>
        </div>
      </header>
      {error && <p role="alert">{error}</p>}
      {loading && <p role="status">Loading dreams…</p>}
      {!loading && !error && runs.length === 0 && (
        <p className="muted">No dreams yet.</p>
      )}
      <ul className="memory-list">
        {runs.map((run) => (
          <li key={run.id} className="memory">
            <div className="memory-meta">
              <strong>
                {run.outcome ? outcomes[run.outcome] : 'Dreaming now'}
              </strong>
              <span className="muted">
                {new Date(run.started_at).toLocaleString()} · {run.model}
              </span>
            </div>
            {run.report && <p>{run.report}</p>}
            {run.revisions.length === 0 ? (
              <p className="muted">No memory changed.</p>
            ) : (
              <ul className="memory-list">
                {run.revisions.map((change) => (
                  <li
                    key={`${change.memory}:${change.revision}`}
                    className="memory"
                  >
                    {change.previous != null && (
                      <p className="muted">
                        <del>{change.previous}</del>
                      </p>
                    )}
                    <p>
                      {change.deleted ? 'Removed from search' : change.content}
                    </p>
                  </li>
                ))}
              </ul>
            )}
          </li>
        ))}
      </ul>
    </section>
  )
}

function listDreams(agent: string) {
  return ok(
    api.GET('/api/agents/{agent}/dreams', { params: { path: { agent } } }),
  )
}
