import { useQueryClient } from '@tanstack/react-query'
import { Link } from '@tanstack/react-router'
import {
  BellOff,
  ChevronRight,
  CircleAlert,
  CircleCheck,
  CircleStop,
  Info,
  MoreHorizontal,
  ShieldAlert,
  ShieldCheck,
  TriangleAlert,
  type LucideIcon,
} from 'lucide-react'
import { useState, type ReactNode } from 'react'
import { admin } from '../../api/admin'
import { api, failure, ok } from '../../api/client'
import type { AgentSummary } from '../../api/types'
import { keys, useAgents, useSession } from '../../app/queries'
import { Cube, moodOf } from '../../components/Cube'
import { PLATFORM } from '../shell/Sidebar'
import {
  Alert,
  confirmAction,
  Empty,
  Menu,
  Skeleton,
  when,
} from '../../components/primitives'
import {
  headline,
  ruleTitle,
  scopeOf,
  SEVERITY,
  severityOf,
  useWatchdog,
  WATCHDOG_KEY,
  type Check,
  type Finding,
  type Mute,
  type Severity,
} from './model'
import './watchdog.css'

type Named = Map<string, AgentSummary>

const MARKS: Record<Severity, LucideIcon> = {
  high: ShieldAlert,
  medium: TriangleAlert,
  low: Info,
}

export function Watchdog() {
  const session = useSession()
  const watchdog = useWatchdog()
  const agents = useAgents()
  const named: Named = new Map(agents.data?.map((agent) => [agent.id, agent]))
  const data = watchdog.data
  if (session.data && !session.data.administrator)
    return (
      <Page title="Watchdog">
        <Empty title="The watchdog is for administrators.">
          It reads every agent’s tool calls, so only people who manage the
          installation see it.
        </Empty>
      </Page>
    )
  return (
    <Page
      title="Watchdog"
      detail={data && headline(data.findings)}
      status={data && <Reading checks={data.checks} />}
    >
      {watchdog.isPending && <Skeleton lines={6} />}
      {watchdog.isError && <Alert>{failure(watchdog.error)}</Alert>}
      {data && (
        <div className="watch">
          <div className="watch__main">
            <p className="watch__lead">
              Reads every tool call, sign-in and change as it is recorded, what
              runs and connects inside each agent’s sandbox, and the pods beside
              it. It never blocks or stops an agent: high and medium findings
              also land in Activity and on your devices, and you decide what
              happens next.
            </p>
            <Open findings={data.findings} named={named} />
            <Dismissed findings={data.dismissed} />
          </div>
          <aside className="watch__side" aria-label="Setup and muted rules">
            <Setup checks={data.checks} />
            <Muted mutes={data.mutes} />
          </aside>
        </div>
      )}
    </Page>
  )
}

function Page({
  title,
  detail,
  status,
  children,
}: {
  title: string
  detail?: string | undefined
  status?: ReactNode
  children: ReactNode
}) {
  return (
    <div className="page">
      <header className="page__head">
        <h1>
          {title}
          {detail && <span className="faint"> · {detail}</span>}
        </h1>
        {status}
      </header>
      <div className="page__body">{children}</div>
    </div>
  )
}

function Reading({ checks }: { checks: Check[] }) {
  const reading = checks.find((check) => check.key === 'watchdog')
  if (!reading) return null
  return (
    <span className="watch-reading" data-ok={reading.ok} title={reading.detail}>
      <span className="watch-reading__dot" aria-hidden="true" />
      {reading.ok ? 'Watching' : 'Behind'}
      <span className="sr-only">: {reading.detail}</span>
    </span>
  )
}

function Section({
  id,
  title,
  count,
  children,
}: {
  id: string
  title: string
  count?: number
  children: ReactNode
}) {
  return (
    <section className="watch__section" aria-labelledby={id}>
      <h2 id={id} className="watch__title">
        {title}
        {count !== undefined && count > 0 && (
          <span className="watch__count">{count}</span>
        )}
      </h2>
      {children}
    </section>
  )
}

function Open({ findings, named }: { findings: Finding[]; named: Named }) {
  return (
    <Section id="watch-open" title="Open" count={findings.length}>
      {findings.length === 0 ? (
        <div className="watch-quiet">
          <ShieldCheck aria-hidden="true" />
          <div>
            <p className="watch-quiet__title">Nothing looks wrong.</p>
            <p>
              A finding appears here within seconds of the tool call, sign-in or
              change that set it off.
            </p>
          </div>
        </div>
      ) : (
        <ul className="findings">
          {findings.map((finding) => (
            <FindingRow
              key={finding.id}
              finding={finding}
              agent={finding.agent ? named.get(finding.agent) : undefined}
            />
          ))}
        </ul>
      )}
    </Section>
  )
}

function Face({
  finding,
  agent,
}: {
  finding: Finding
  agent: AgentSummary | undefined
}) {
  if (finding.agent)
    return (
      <span className="finding__face" aria-hidden="true">
        <Cube
          agentId={finding.agent}
          avatar={agent?.avatar ?? null}
          mood={agent ? moodOf(agent) : 'idle'}
          size={30}
        />
      </span>
    )
  return (
    <span
      className="finding__face finding__face--installation"
      aria-hidden="true"
    >
      <ShieldAlert />
    </span>
  )
}

function useSettle() {
  const client = useQueryClient()
  return () =>
    Promise.all([
      client.invalidateQueries({ queryKey: WATCHDOG_KEY }),
      client.invalidateQueries({ queryKey: keys.activity }),
    ])
}

function FindingRow({
  finding,
  agent,
}: {
  finding: Finding
  agent: AgentSummary | undefined
}) {
  const settle = useSettle()
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  const severity = severityOf(finding)
  const Mark = MARKS[severity]
  const name = finding.agent_name ?? agent?.name
  async function act(work: () => Promise<unknown>) {
    setBusy(true)
    setError('')
    try {
      await work()
      await settle()
    } catch (caught) {
      setError(failure(caught))
    }
    setBusy(false)
  }
  const dismiss = (mute: boolean) =>
    act(() =>
      ok(
        api.PUT('/api/watchdog/findings/{finding}/dismiss', {
          params: { path: { finding: finding.id } },
          body: { mute },
        }),
      ),
    )
  return (
    <li className="finding" data-severity={severity} aria-busy={busy}>
      <Face finding={finding} agent={agent} />
      <div className="finding__body">
        <p className="finding__head">
          <Who finding={finding} name={name} />
          <span className="finding__severity">
            <Mark aria-hidden="true" />
            {SEVERITY[severity]}
          </span>
          <span className="finding__rule">{ruleTitle(finding.rule)}</span>
          {finding.count > 1 && (
            <span
              className="finding__count tabular"
              title="Times it matched while open"
            >
              ×{finding.count}
            </span>
          )}
          <time dateTime={finding.seen_at} title={full(finding.seen_at)}>
            {when(finding.seen_at)}
          </time>
        </p>
        <p className="finding__summary">{finding.summary}</p>
        {finding.evidence && <Evidence text={finding.evidence} />}
        <Alert>{error}</Alert>
      </div>
      <div className="finding__actions">
        {finding.agent && finding.turn && (
          <Link
            to="/agents/$agentId/trace/$turnId"
            params={{ agentId: finding.agent, turnId: finding.turn }}
            className="btn btn--sm"
          >
            Open trace
          </Link>
        )}
        <button
          type="button"
          className="btn btn--ghost btn--sm"
          disabled={busy}
          onClick={() => void dismiss(false)}
        >
          Dismiss
        </button>
        <Menu
          className="menu--right finding__more"
          label={`More for ${ruleTitle(finding.rule)}`}
          trigger={<MoreHorizontal />}
          triggerClassName="btn btn--ghost btn--icon btn--sm"
        >
          <button
            type="button"
            className="menu__item"
            disabled={busy}
            onClick={() => void dismiss(true)}
          >
            <BellOff /> Dismiss and mute{' '}
            {finding.agent ? scopeOf(name ?? 'this agent') : scopeOf(null)}
          </button>
          {agent?.desired_state === 'running' && (
            <StopAgent agent={agent} act={act} busy={busy} />
          )}
        </Menu>
      </div>
    </li>
  )
}

function Who({
  finding,
  name,
}: {
  finding: Finding
  name: string | null | undefined
}) {
  if (!finding.agent) return <span className="finding__name">Installation</span>
  return (
    <Link
      to="/agents/$agentId"
      params={{ agentId: finding.agent }}
      className="finding__name"
    >
      {name ?? 'An agent'}
    </Link>
  )
}

function StopAgent({
  agent,
  act,
  busy,
}: {
  agent: AgentSummary
  act: (work: () => Promise<unknown>) => Promise<void>
  busy: boolean
}) {
  const client = useQueryClient()
  async function stop() {
    const yes = await confirmAction({
      title: `Stop ${agent.name}?`,
      body: 'Its sandbox stops once its durable state is saved; open terminal jobs and browser sessions end. A new message or routine starts it again.',
      action: 'Stop',
    })
    if (!yes) return
    await act(async () => {
      await admin.lifecycle(agent.id, 'stop', agent.revision)
      await client.invalidateQueries({ queryKey: keys.agents })
    })
  }
  return (
    <button
      type="button"
      className="menu__item menu__item--danger"
      disabled={busy}
      onClick={() => void stop()}
    >
      <CircleStop /> Stop {agent.name}
    </button>
  )
}

function Evidence({ text }: { text: string }) {
  return (
    <details className="finding__evidence">
      <summary>
        <ChevronRight aria-hidden="true" />
        What matched
      </summary>
      <pre>{text}</pre>
    </details>
  )
}

function Dismissed({ findings }: { findings: Finding[] }) {
  if (findings.length === 0) return null
  return (
    <details className="disclosure watch__dismissed">
      <summary>
        Dismissed in the last 30 days{' '}
        <span className="chip tabular">{findings.length}</span>
      </summary>
      <ul className="settled">
        {findings.map((finding) => (
          <li key={finding.id} className="settled__row">
            <span className="settled__rule">{ruleTitle(finding.rule)}</span>
            <span className="settled__summary">
              {finding.agent_name ? `${finding.agent_name}: ` : ''}
              {finding.summary}
            </span>
            <span className="settled__by">
              {finding.dismissed_by ?? 'Someone'} ·{' '}
              <time
                dateTime={finding.dismissed_at ?? undefined}
                title={full(finding.dismissed_at)}
              >
                {when(finding.dismissed_at)}
              </time>
            </span>
          </li>
        ))}
      </ul>
    </details>
  )
}

function Setup({ checks }: { checks: Check[] }) {
  const shown = checks.filter((check) => check.key !== 'watchdog')
  const review = shown.filter((check) => !check.ok).length
  return (
    <Section id="watch-setup" title="Setup">
      <p className="watch__note">
        {review === 0
          ? 'Nothing here widens what one misled agent could do.'
          : `${review} ${review === 1 ? 'thing widens' : 'things widen'} what one misled agent could do.`}
      </p>
      <ul className="posture">
        {shown.map((check) => (
          <li key={check.key} className="posture__row" data-ok={check.ok}>
            {check.ok ? (
              <CircleCheck className="posture__mark" aria-label="Sound" />
            ) : (
              <CircleAlert
                className="posture__mark"
                aria-label="Worth a look"
              />
            )}
            <div className="posture__body">
              <p className="posture__title">{check.title}</p>
              <p className="posture__detail">{check.detail}</p>
            </div>
            <Review to={check.ok ? null : check.to} />
          </li>
        ))}
      </ul>
    </Section>
  )
}

/** A link to the platform page that changes a check; only pages the console knows. */
function Review({ to }: { to: string | null | undefined }) {
  const page = PLATFORM.find(([path]) => path === to)?.[0]
  if (!page) return null
  return (
    <Link to={page} className="link-btn posture__go">
      Review
    </Link>
  )
}

function Muted({ mutes }: { mutes: Mute[] }) {
  return (
    <Section id="watch-muted" title="Muted" count={mutes.length}>
      {mutes.length === 0 ? (
        <p className="watch__note">
          No rules are muted. Dismiss and mute a finding to stop its rule
          opening more for that agent.
        </p>
      ) : (
        <ul className="posture">
          {mutes.map((mute) => (
            <MuteRow key={mute.id} mute={mute} />
          ))}
        </ul>
      )}
    </Section>
  )
}

function MuteRow({ mute }: { mute: Mute }) {
  const settle = useSettle()
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  async function unmute() {
    setBusy(true)
    setError('')
    try {
      await ok(
        api.DELETE('/api/watchdog/mutes/{mute}', {
          params: { path: { mute: mute.id } },
        }),
      )
      await settle()
    } catch (caught) {
      setError(failure(caught))
      setBusy(false)
    }
  }
  return (
    <li className="posture__row">
      <BellOff
        className="posture__mark posture__mark--muted"
        aria-hidden="true"
      />
      <div className="posture__body">
        <p className="posture__title">
          {ruleTitle(mute.rule)}{' '}
          <span className="faint">
            {mute.agent
              ? scopeOf(mute.agent_name ?? 'an agent')
              : scopeOf(null)}
          </span>
        </p>
        <p className="posture__detail">
          Muted by {mute.muted_by} · {when(mute.muted_at)}
        </p>
        <Alert>{error}</Alert>
      </div>
      <button
        type="button"
        className="btn btn--sm"
        disabled={busy}
        onClick={() => void unmute()}
      >
        {busy ? 'Unmuting…' : 'Unmute'}
      </button>
    </li>
  )
}

function full(at: string | null | undefined) {
  return at ? new Date(at).toLocaleString() : undefined
}
