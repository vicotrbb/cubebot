import { useQuery } from '@tanstack/react-query'
import { Link } from '@tanstack/react-router'
import {
  BookOpen,
  Brain,
  CircleAlert,
  Library,
  MessageSquareQuote,
  NotebookPen,
  ShieldCheck,
  Sparkles,
} from 'lucide-react'
import type { ReactNode } from 'react'
import { api, ok } from '../../api/client'
import { Alert, CopyButton, Skeleton } from '../../components/primitives'
import { count, money } from '../home/format'
import { PROVIDER_NAMES } from '../platform/models'
import {
  pulledOf,
  stepsOf,
  totalsOf,
  type Pulled,
  type Step,
  type Totals,
  type Trace,
} from './trace'
import { WorkCard } from './WorkCard'
import './trace.css'

const STAGES: Record<string, string> = {
  reply: 'Model call',
  review: 'Finishing review',
  memory: 'Memory extraction',
  summary: 'Context summary',
}

const REVIEWS: Record<string, string> = {
  asked: 'Asked the model to review its work against the request',
  complete: 'Review found the work complete; the reply stands',
  revised: 'Review changed the reply',
}

const LIVE = new Set(['queued', 'running'])

export function TracePanel({
  agentId,
  turnId,
}: {
  agentId: string
  turnId: string
}) {
  const trace = useQuery({
    queryKey: ['trace', agentId, turnId],
    queryFn: () =>
      ok(
        api.GET('/api/conversations/{conversation}/messages/{turn}/trace', {
          params: { path: { conversation: agentId, turn: turnId } },
        }),
      ),
    refetchInterval: (query) =>
      LIVE.has(query.state.data?.status ?? '') ? 2_000 : false,
  })
  if (trace.isPending) return <Skeleton lines={8} />
  if (trace.isError)
    return <Alert>The trace could not be loaded. Retry shortly.</Alert>
  return <TraceView trace={trace.data} agentId={agentId} />
}

function TraceView({ trace, agentId }: { trace: Trace; agentId: string }) {
  const steps = stepsOf(trace)
  const totals = totalsOf(steps)
  const pulled = pulledOf(steps)
  return (
    <div className="trace">
      <header className="trace__head">
        <div>
          <h2>Trace</h2>
          <p className="trace__meta">{metaOf(trace)}</p>
        </div>
        <CopyButton
          text={JSON.stringify(trace, null, 2)}
          label="Copy the trace as JSON"
        />
      </header>
      <TotalsRow totals={totals} />
      <section className="trace__section" aria-labelledby="trace-pulled">
        <h3 id="trace-pulled">Pulled into context</h3>
        <PulledList pulled={pulled} agentId={agentId} />
      </section>
      {trace.memories.length > 0 && (
        <section className="trace__section" aria-labelledby="trace-saved">
          <h3 id="trace-saved">Memories saved</h3>
          <ul className="trace__list">
            {trace.memories.map((memory) => (
              <li key={`${memory.id}-${memory.revision}`}>
                <NotebookPen aria-hidden="true" />
                <span>
                  {memory.deleted ? (
                    <span className="trace__tag">Forgotten</span>
                  ) : memory.revision > 1 ? (
                    <span className="trace__tag">Revised</span>
                  ) : null}
                  {memory.content}
                </span>
              </li>
            ))}
          </ul>
        </section>
      )}
      <section className="trace__section" aria-labelledby="trace-steps">
        <h3 id="trace-steps">Steps</h3>
        {steps.length === 0 ? (
          <p className="panel__lead">
            {LIVE.has(trace.status)
              ? 'Nothing recorded yet. Steps appear as the agent works.'
              : 'No steps were recorded for this reply.'}
          </p>
        ) : (
          <ol className="trace__steps">
            {steps.map((step, index) => (
              <StepRow key={index} step={step} />
            ))}
          </ol>
        )}
      </section>
    </div>
  )
}

function metaOf(trace: Trace) {
  const started = new Date(trace.created_at)
  const parts = [
    trace.model,
    trace.provider
      ? (PROVIDER_NAMES[trace.provider as keyof typeof PROVIDER_NAMES] ??
        trace.provider)
      : null,
    started.toLocaleString(undefined, {
      dateStyle: 'medium',
      timeStyle: 'short',
    }),
    trace.completed_at
      ? `took ${elapsed(Date.parse(trace.completed_at) - started.getTime())}`
      : LIVE.has(trace.status)
        ? 'working'
        : trace.status,
  ]
  return parts.filter(Boolean).join(' · ')
}

function elapsed(ms: number) {
  const seconds = Math.max(0, Math.round(ms / 1000))
  if (seconds < 60) return `${seconds}s`
  const minutes = Math.floor(seconds / 60)
  return `${minutes}m ${String(seconds % 60).padStart(2, '0')}s`
}

function offset(ms: number) {
  const seconds = Math.max(0, Math.round(ms / 1000))
  if (seconds < 60) return `${seconds}s`
  return `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, '0')}`
}

function TotalsRow({ totals }: { totals: Totals }) {
  return (
    <dl className="trace__totals">
      <div>
        <dt>Model calls</dt>
        <dd className="tabular">{totals.calls}</dd>
      </div>
      <div>
        <dt>Tool calls</dt>
        <dd className="tabular">{totals.tools}</dd>
      </div>
      <div>
        <dt>Tokens</dt>
        <dd className="tabular">
          {count(totals.input)} in · {count(totals.output)} out
          {totals.cached > 0 && (
            <span className="faint">{count(totals.cached)} from cache</span>
          )}
        </dd>
      </div>
      <div>
        <dt>Cost</dt>
        <dd className="tabular">
          {totals.calls === 0
            ? 'None'
            : totals.unpriced && totals.cost === 0
              ? 'Unpriced'
              : money(totals.cost)}
          {totals.unpriced && totals.cost > 0 && (
            <span className="faint">plus unpriced calls</span>
          )}
        </dd>
      </div>
    </dl>
  )
}

function PulledList({
  pulled,
  agentId,
}: {
  pulled: Pulled[]
  agentId: string
}) {
  if (pulled.length === 0)
    return (
      <p className="panel__lead">
        Nothing. The agent worked from the conversation and its standing
        instructions without searching its memory, the knowledge base or its
        skills.
      </p>
    )
  return (
    <ul className="trace__list">
      {pulled.map((item, index) => (
        <li key={index}>
          <PulledItem item={item} agentId={agentId} />
        </li>
      ))}
    </ul>
  )
}

function PulledItem({ item, agentId }: { item: Pulled; agentId: string }) {
  if (item.kind === 'skill')
    return (
      <>
        <Sparkles aria-hidden="true" />
        <span>
          Loaded the skill <strong>{item.name}</strong>
        </span>
      </>
    )
  if (item.kind === 'kb_page')
    return (
      <>
        <BookOpen aria-hidden="true" />
        <span>
          Read{' '}
          <Link to="/kb/$pageId" params={{ pageId: item.id }}>
            {item.title || 'a knowledge base page'}
          </Link>
        </span>
      </>
    )
  if (item.kind === 'kb_search')
    return (
      <>
        <Library aria-hidden="true" />
        <span>
          Searched the knowledge base for <q>{item.query}</q>
          <Found
            empty="No pages matched."
            items={item.found.map((page) => ({
              id: page.id,
              node: (
                <Link to="/kb/$pageId" params={{ pageId: page.id }}>
                  {page.title || 'Untitled'}
                </Link>
              ),
            }))}
          />
        </span>
      </>
    )
  return (
    <>
      <Brain aria-hidden="true" />
      <span>
        Searched memory for <q>{item.query}</q>
        <Found
          empty="No memories matched."
          items={item.found.map((memory) => ({
            id: memory.id,
            node: (
              <Link
                to="/agents/$agentId/memories"
                params={{ agentId }}
                className="trace__memory"
              >
                {memory.content}
              </Link>
            ),
          }))}
        />
      </span>
    </>
  )
}

function Found({
  items,
  empty,
}: {
  items: { id: string; node: ReactNode }[]
  empty: string
}) {
  if (items.length === 0) return <span className="trace__none">{empty}</span>
  return (
    <ul className="trace__found">
      {items.map((item, index) => (
        <li key={`${item.id}-${index}`}>{item.node}</li>
      ))}
    </ul>
  )
}

function StepRow({ step }: { step: Step }) {
  return (
    <li className="trace__step" data-kind={step.kind}>
      <span className="trace__at tabular" aria-label="Time into the reply">
        +{offset(step.at)}
      </span>
      <div className="trace__what">
        <StepBody step={step} />
      </div>
    </li>
  )
}

function StepBody({ step }: { step: Step }) {
  switch (step.kind) {
    case 'model':
      return (
        <p className="trace__line">
          <strong>{STAGES[step.stage]}</strong>
          <span className="tabular faint">
            {count(step.input)} in
            {step.cached > 0 && ` (${count(step.cached)} cached)`} ·{' '}
            {count(step.output)} out ·{' '}
            {step.cost === null ? 'unpriced' : money(step.cost)}
          </span>
        </p>
      )
    case 'tool':
      return (
        <>
          <WorkCard tool={step.tool} />
          {step.took !== null && (
            <span className="trace__took tabular">{elapsed(step.took)}</span>
          )}
        </>
      )
    case 'review':
      return (
        <p className="trace__line" data-outcome={step.outcome}>
          <ShieldCheck aria-hidden="true" />
          <span>{REVIEWS[step.outcome]}</span>
        </p>
      )
    case 'steered':
      return (
        <p className="trace__line">
          <MessageSquareQuote aria-hidden="true" />
          <span>
            Steered: <q>{step.message}</q>
          </span>
        </p>
      )
    case 'summary':
      return (
        <details className="trace__summary">
          <summary>Context summarized to stay within the window</summary>
          <p>{step.text}</p>
        </details>
      )
    default:
      return (
        <p className="trace__line trace__line--error">
          <CircleAlert aria-hidden="true" />
          <span>Stopped: {step.code.replaceAll('_', ' ')}</span>
        </p>
      )
  }
}
