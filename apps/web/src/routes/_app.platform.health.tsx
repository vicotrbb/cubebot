import { useQuery } from '@tanstack/react-query'
import { createFileRoute, Link } from '@tanstack/react-router'
import { admin } from '../api/admin'
import { useAgents, useInstallation } from '../app/queries'
import { Cube, moodOf } from '../components/Cube'
import { sandboxLabel } from '../features/platform/sandbox'
import '../features/platform/platform.css'

export const Route = createFileRoute('/_app/platform/health')({
  component: Health,
})

/** What the orchestrator reports about itself; not a claim about qualification. */
function Health() {
  const installation = useInstallation()
  const agents = useAgents()
  const ready = useQuery({
    queryKey: ['ready'],
    queryFn: () => admin.ready().then((response) => response.ok),
    refetchInterval: 15_000,
  })
  const byState = (agents.data ?? []).reduce<Record<string, number>>(
    (acc, agent) => {
      acc[agent.activity] = (acc[agent.activity] ?? 0) + 1
      return acc
    },
    {},
  )
  return (
    <div className="platform">
      <div>
        <p className="platform__lead">
          Readiness checks the database only; it says nothing about agents,
          Kubernetes or providers. Each agent's Status tab shows its sandbox.
        </p>
        <dl className="kv">
          <dt>Orchestrator</dt>
          <dd>
            {ready.data === undefined
              ? 'checking…'
              : ready.data
                ? 'ready'
                : 'not ready'}
          </dd>
          <dt>Controller</dt>
          <dd>{installation.data?.controller.replaceAll('_', ' ')}</dd>
          <dt>Sandbox</dt>
          <dd>{installation.data && sandboxLabel(installation.data)}</dd>
          <dt>Terminal tools</dt>
          <dd>{installation.data?.terminal_enabled ? 'on' : 'off'}</dd>
          <dt>Computer use</dt>
          <dd>{installation.data?.computer_use_enabled ? 'on' : 'off'}</dd>
          <dt>Agents</dt>
          <dd>
            {agents.data?.length ?? 0} registered
            {Object.entries(byState).map(([state, count]) => (
              <span key={state} className="chip" style={{ marginLeft: 6 }}>
                {count} {state}
              </span>
            ))}
          </dd>
        </dl>
        <h3 style={{ margin: '24px 0 8px' }}>Agents</h3>
        <table className="table">
          <thead>
            <tr>
              <th>Agent</th>
              <th>Activity</th>
              <th>Desired</th>
              <th>Lanes</th>
              <th />
            </tr>
          </thead>
          <tbody>
            {agents.data?.map((agent) => (
              <tr key={agent.id}>
                <td>
                  <span className="actions" style={{ gap: 8 }}>
                    <Cube
                      agentId={agent.id}
                      avatar={agent.avatar}
                      mood={moodOf(agent)}
                      size={20}
                    />
                    {agent.name}
                  </span>
                </td>
                <td>
                  <span className={`dot dot--${agent.activity}`} />{' '}
                  {agent.activity}
                </td>
                <td>{agent.desired_state}</td>
                <td className="num">{agent.task_lanes}</td>
                <td className="num">
                  <Link
                    to="/agents/$agentId/status"
                    params={{ agentId: agent.id }}
                    className="small"
                  >
                    Status
                  </Link>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      <aside className="platform__editor">
        <h2>Not covered here</h2>
        <p className="small muted">
          Backups, upgrades and capacity limits have no console view yet.
        </p>
      </aside>
    </div>
  )
}
