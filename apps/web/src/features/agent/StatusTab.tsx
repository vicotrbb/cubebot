import { useQuery, useQueryClient } from '@tanstack/react-query'
import { RefreshCw } from 'lucide-react'
import { admin, type AgentStatus } from '../../api/admin'
import type { AgentSummary } from '../../api/types'
import { keys } from '../../app/queries'
import { Alert, Skeleton, when } from '../../components/primitives'
import { Lifecycle } from './About'

export function StatusTab({ agent }: { agent: AgentSummary }) {
  const client = useQueryClient()
  const status = useQuery({
    queryKey: ['agent-status', agent.id],
    queryFn: () => admin.agentStatus(agent.id),
    refetchInterval: 10_000,
  })
  return (
    <>
      <section className="panel__section">
        <div className="actions" style={{ justifyContent: 'space-between' }}>
          <h2>Status</h2>
          <button
            type="button"
            className="btn btn--ghost btn--icon btn--sm"
            aria-label="Refresh"
            onClick={() => void status.refetch()}
          >
            <RefreshCw />
          </button>
        </div>
        {status.isPending && <Skeleton lines={4} />}
        {status.isError && <Alert>The status could not be loaded.</Alert>}
        {status.data && <Rows s={status.data} activity={agent.activity} />}
      </section>
      <Lifecycle
        agent={agent}
        onChanged={async () => {
          await client.invalidateQueries({ queryKey: keys.agents })
          await status.refetch()
        }}
      />
    </>
  )
}

function Rows({ s, activity }: { s: AgentStatus; activity: string }) {
  return (
    <dl className="kv">
      <dt>Desired</dt>
      <dd>{s.desired_state}</dd>
      <dt>Activity</dt>
      <dd>{activity}</dd>
      <dt>Sandbox</dt>
      <dd>{sandbox(s)}</dd>
      <dt>Kubernetes</dt>
      <dd>{kubernetes(s)}</dd>
      <dt>Observed</dt>
      <dd>
        {s.observed_at ? when(s.observed_at) : 'never'}
        {s.observed_seconds_ago != null &&
          ` (${Math.round(s.observed_seconds_ago)} s ago)`}
      </dd>
      {s.blocked_reason && (
        <>
          <dt>Blocked</dt>
          <dd style={{ color: 'var(--danger)' }}>
            {s.blocked_reason.replaceAll('_', ' ')}
          </dd>
        </>
      )}
      <dt>Restarts</dt>
      <dd>generation {s.restart_generation}</dd>
      <dt>Evidence</dt>
      <dd>
        {s.terminal_evidence_retained
          ? 'terminal evidence retained'
          : 'none retained'}
      </dd>
      <dt>Revision</dt>
      <dd>{s.revision}</dd>
    </dl>
  )
}

function sandbox(s: AgentStatus) {
  const parts = [s.resource_phase ?? 'none']
  if (s.pod_epoch != null) parts.push(`epoch ${s.pod_epoch}`)
  if (s.pod_attempt) parts.push(`attempt ${s.pod_attempt.slice(0, 8)}`)
  return parts.join(' · ')
}

function kubernetes(s: AgentStatus) {
  const parts = [s.kubernetes_phase ?? 'not observed']
  if (s.kubernetes_ready != null)
    parts.push(s.kubernetes_ready ? 'ready' : 'not ready')
  if (s.kubernetes_reason) parts.push(s.kubernetes_reason)
  if (s.deleting) parts.push('deleting')
  return parts.join(' · ')
}
