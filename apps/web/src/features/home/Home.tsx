import { useQuery } from '@tanstack/react-query'
import { Link } from '@tanstack/react-router'
import {
  BookOpen,
  CircleCheck,
  CircleX,
  List,
  Moon,
  Network,
  PanelLeft,
  type LucideIcon,
} from 'lucide-react'
import { useState } from 'react'
import { admin } from '../../api/admin'
import type { components } from '../../api/schema'
import type { AgentSummary } from '../../api/types'
import {
  notRetired,
  useAgents,
  useInstallation,
  useSituation,
  useTeams,
} from '../../app/queries'
import { Cube, moodOf } from '../../components/Cube'
import { Alert, Empty, Skeleton, when } from '../../components/primitives'
import { doingOf } from '../agent/doing'
import { fileSize } from '../files/viewer'
import { sandboxLabel } from '../platform/sandbox'
import { useIsPhone } from '../shell/viewport'
import { OrgChart } from '../teams/OrgChart'
import { DayBars } from './DayBars'
import { count, headline, money, pace } from './format'
import './home.css'

type Data = components['schemas']['Situation']
type Row = components['schemas']['SituationAgent']
type Recent = components['schemas']['RecentEvent']
type Open = components['schemas']['OpenWork']

type View = 'list' | 'org'

/** The console's landing page: every agent at a glance, with spend and work kept to the side. */
export function Home({
  view,
  onView,
}: {
  view: View
  onView: (view: View) => void
}) {
  const situation = useSituation()
  const agents = useAgents()
  const phone = useIsPhone()
  return (
    <div className="page home">
      <header className="page__head">
        <h1>Home</h1>
        <span className="home__live small faint">
          <span className="dot dot--working" aria-hidden="true" /> Live
        </span>
        {phone && (
          <Link
            to="/menu"
            className="btn btn--sm home__menu"
            aria-label="Agents, teams and rooms"
          >
            <PanelLeft />
            <span>Browse</span>
          </Link>
        )}
      </header>
      <div className="page__body home__body">
        {situation.isError && (
          <Alert>Home could not be loaded. Retrying shortly.</Alert>
        )}
        {situation.isPending && <Skeleton lines={6} width="60%" />}
        {situation.data && (
          <Board
            data={situation.data}
            agents={notRetired(agents.data)}
            view={view}
            onView={onView}
          />
        )}
      </div>
    </div>
  )
}

function Board({
  data,
  agents,
  view,
  onView,
}: {
  data: Data
  agents: AgentSummary[]
  view: View
  onView: (view: View) => void
}) {
  const [pane, setPane] = useState<(typeof PANES)[number][0]>('agents')
  const named = new Map(agents.map((agent) => [agent.id, agent]))
  const rows = data.agents.filter((row) => named.has(row.id))
  const line = headline({
    agents: rows.flatMap((row) => named.get(row.id) ?? []),
    work: data.work,
    failedToday: rows.reduce((sum, row) => sum + row.failed_today, 0),
    overBudget: rows.filter(
      (row) => pace(row.cost_month_usd, row.monthly_budget_usd) === 'over',
    ).length,
  })
  if (rows.length === 0)
    return (
      <Empty title="No agents to watch yet.">
        Agents you can read appear here with what they are doing and spending.
      </Empty>
    )
  return (
    <div className="home__grid" data-pane={pane}>
      <section className="brief" aria-label="Summary" data-calm={line.calm}>
        <h2>{line.text}</h2>
        <p>{line.state}</p>
      </section>
      <div className="tabs home__panes" role="group" aria-label="Show">
        {PANES.map(([id, label]) => (
          <button
            key={id}
            type="button"
            aria-pressed={pane === id}
            onClick={() => setPane(id)}
          >
            {label}
          </button>
        ))}
      </div>
      <AgentsBoard rows={rows} named={named} view={view} onView={onView} />
      <aside className="home__rail" aria-label="Spend and work">
        <Spend data={data} />
        <NowList work={data.work} named={named} />
        <RecentList recent={data.recent} named={named} />
      </aside>
      <Platform data={data} />
    </div>
  )
}

const PANES = [
  ['agents', 'Agents'],
  ['now', 'Work'],
  ['spend', 'Spend'],
] as const

const VIEWS: [View, string, LucideIcon][] = [
  ['list', 'List', List],
  ['org', 'Org chart', Network],
]

function AgentsBoard({
  rows,
  named,
  view,
  onView,
}: {
  rows: Row[]
  named: Map<string, AgentSummary>
  view: View
  onView: (view: View) => void
}) {
  const teams = useTeams()
  return (
    <section
      className="board-section board-section--agents"
      aria-labelledby="agents"
    >
      <div className="board-section__head">
        <h3 id="agents">Agents</h3>
        <div className="tabs" role="group" aria-label="Show agents as">
          {VIEWS.map(([id, label, Icon]) => (
            <button
              key={id}
              type="button"
              aria-pressed={view === id}
              onClick={() => onView(id)}
            >
              <Icon aria-hidden="true" />
              {label}
            </button>
          ))}
        </div>
      </div>
      {view === 'list' ? (
        <AgentsLedger rows={rows} named={named} />
      ) : teams.isPending ? (
        <Skeleton lines={4} width="50%" />
      ) : (
        <div
          className="board-section__scroll"
          tabIndex={0}
          aria-label="Org chart"
        >
          <OrgChart teams={teams.data ?? []} agents={[...named.values()]} />
        </div>
      )}
    </section>
  )
}

function Spend({ data }: { data: Data }) {
  const [measure, setMeasure] = useState<'cost_usd' | 'tokens'>('cost_usd')
  const unpriced = data.month.unpriced
  return (
    <section
      className="board-section board-section--spend"
      aria-labelledby="spend"
    >
      <div className="board-section__head">
        <h3 id="spend">Spend</h3>
        <div className="tabs" role="group" aria-label="Measure">
          <button
            type="button"
            aria-pressed={measure === 'cost_usd'}
            onClick={() => setMeasure('cost_usd')}
          >
            Cost
          </button>
          <button
            type="button"
            aria-pressed={measure === 'tokens'}
            onClick={() => setMeasure('tokens')}
          >
            Tokens
          </button>
        </div>
      </div>
      <dl className="figures">
        <Figure name="Today" spend={data.today} />
        <Figure name="7 days" spend={data.week} />
        <Figure name="This month" spend={data.month} />
      </dl>
      <DayBars
        days={data.days}
        measure={measure}
        format={measure === 'cost_usd' ? money : count}
        label={measure === 'cost_usd' ? 'Estimated cost' : 'Tokens'}
      />
      {data.models.length > 0 && (
        <details className="spend__models">
          <summary>Models, last 30 days</summary>
          <table className="table models">
            <thead>
              <tr>
                <th scope="col">Model</th>
                <th scope="col" className="num">
                  Tokens
                </th>
                <th scope="col" className="num">
                  Cost
                </th>
              </tr>
            </thead>
            <tbody>
              {data.models.map((model) => (
                <tr key={`${model.provider}/${model.model}`}>
                  <td className="models__name">{model.model}</td>
                  <td className="num">{count(model.tokens)}</td>
                  <td className="num">{modelCost(model)}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </details>
      )}
      <p className="board-section__note">
        Estimates from each model&rsquo;s listed price, in UTC
        {data.prices_at ? '' : '; no price list has loaded yet'}.
        {unpriced > 0 &&
          ` ${count(unpriced)} ${unpriced === 1 ? 'response' : 'responses'} this month had no price and count only as tokens.`}
      </p>
    </section>
  )
}

/** A model with tokens and no cost has no known price; $0.00 would say it was free. */
export function modelCost(model: { tokens: number; cost_usd: number }) {
  return model.cost_usd === 0 && model.tokens > 0
    ? 'No price'
    : money(model.cost_usd)
}

function Figure({
  name,
  spend,
}: {
  name: string
  spend: components['schemas']['Spend']
}) {
  return (
    <div>
      <dt>{name}</dt>
      <dd>
        <strong>{money(spend.cost_usd)}</strong>
        <span>{count(spend.input_tokens + spend.output_tokens)} tokens</span>
      </dd>
    </div>
  )
}

function AgentsLedger({
  rows,
  named,
}: {
  rows: Row[]
  named: Map<string, AgentSummary>
}) {
  return (
    <div className="board-section__scroll" tabIndex={0} aria-label="Agents">
      <table className="table ledger">
        <thead>
          <tr>
            <th scope="col">Agent</th>
            <th scope="col" className="num ledger__wide">
              Open
            </th>
            <th scope="col" className="num ledger__wide">
              7 days
            </th>
            <th scope="col" className="ledger__money">
              This month
            </th>
          </tr>
        </thead>
        <tbody>
          {rows.map((row) => {
            const agent = named.get(row.id)
            return agent ? (
              <AgentRow key={row.id} row={row} agent={agent} />
            ) : null
          })}
        </tbody>
      </table>
    </div>
  )
}

function AgentRow({ row, agent }: { row: Row; agent: AgentSummary }) {
  return (
    <tr>
      <td>
        <Link
          to="/agents/$agentId/usage"
          params={{ agentId: agent.id }}
          className="ledger__agent"
        >
          <Cube
            agentId={agent.id}
            avatar={agent.avatar}
            mood={moodOf(agent)}
            size={28}
          />
          <span>
            <strong>{agent.name}</strong>
            <small>
              <span
                className={`dot dot--${agent.activity}`}
                aria-hidden="true"
              />{' '}
              {doingOf(agent)}
              {row.failed_today > 0 && (
                <em> · {row.failed_today} failed today</em>
              )}
            </small>
          </span>
        </Link>
      </td>
      <td className="num ledger__wide">{row.open_turns}</td>
      <td className="num ledger__wide">
        {count(row.tokens_week)}
        <small>{money(row.cost_week_usd)}</small>
      </td>
      <td className="ledger__money">
        <BudgetMeter
          spent={row.cost_month_usd}
          budget={row.monthly_budget_usd}
        />
      </td>
    </tr>
  )
}

export function BudgetMeter({
  spent,
  budget,
  period = 'monthly',
}: {
  spent: number
  budget: number | null | undefined
  period?: 'daily' | 'monthly'
}) {
  const state = pace(spent, budget)
  if (state === 'none' || !budget)
    return (
      <div className="meter">
        <strong>{money(spent)}</strong>
        <small>No limit</small>
      </div>
    )
  const WORDS = { fine: 'of', near: 'nearly at', over: 'over' }
  return (
    <div className="meter" data-state={state}>
      <strong>{money(spent)}</strong>
      <small>
        {WORDS[state]} {money(budget)}
      </small>
      <span
        className="meter__track"
        role="meter"
        aria-label={`Share of ${period} budget spent`}
        aria-valuemin={0}
        aria-valuemax={budget}
        aria-valuenow={Math.min(spent, budget)}
      >
        <span style={{ transform: `scaleX(${Math.min(1, spent / budget)})` }} />
      </span>
    </div>
  )
}

const OPEN: Record<string, string> = {
  queued: 'Queued',
  running: 'Working',
  delegated: 'Waiting on delegates',
  waiting: 'Waiting for you',
}

function workLink(item: { room?: string | null; agent: string }) {
  return item.room ? `/rooms/${item.room}` : `/agents/${item.agent}`
}

function NowList({
  work,
  named,
}: {
  work: Open[]
  named: Map<string, AgentSummary>
}) {
  return (
    <section className="board-section board-section--now" aria-labelledby="now">
      <div className="board-section__head">
        <h3 id="now">Now</h3>
        {work.length > 0 && (
          <span className="board-section__count">{work.length}</span>
        )}
      </div>
      {work.length === 0 && (
        <p className="board-section__note">Nothing is running.</p>
      )}
      <ul className="feed" tabIndex={0} aria-label="Open work">
        {work.map((item) => (
          <li key={item.turn} data-status={item.status}>
            <Link to={workLink(item)}>
              <span className="feed__line">
                <strong>{named.get(item.agent)?.name ?? 'An agent'}</strong>{' '}
                <span className="feed__state">
                  {item.error === 'budget_exhausted'
                    ? 'Paused by its budget'
                    : item.error === 'provider_usage_limit'
                      ? 'Waiting for plan usage'
                      : (OPEN[item.status] ?? item.status)}
                </span>
              </span>
              <span className="feed__text">{preview(item.message)}</span>
            </Link>
            <time dateTime={item.since}>{when(item.since)}</time>
          </li>
        ))}
      </ul>
    </section>
  )
}

function preview(text: string) {
  return text
    .replace(/[*_`#>~]+/g, '')
    .replace(/\s+/g, ' ')
    .trim()
}

const RECENT: Record<string, [LucideIcon, string]> = {
  turn_complete: [CircleCheck, 'finished'],
  turn_failed: [CircleX, 'failed'],
  turn_cancelled: [CircleX, 'was stopped'],
  dream: [Moon, 'dreamed'],
  skill: [BookOpen, 'wrote a skill'],
}

function RecentList({
  recent,
  named,
}: {
  recent: Recent[]
  named: Map<string, AgentSummary>
}) {
  return (
    <section
      className="board-section board-section--latest"
      aria-labelledby="latest"
    >
      <div className="board-section__head">
        <h3 id="latest">Latest</h3>
      </div>
      {recent.length === 0 && (
        <p className="board-section__note">Nothing has happened yet.</p>
      )}
      <ul className="feed" tabIndex={0} aria-label="Latest events">
        {recent.map((item, index) => {
          const [Icon, verb] = RECENT[item.kind] ?? [CircleCheck, item.kind]
          return (
            <li key={`${item.kind}:${item.at}:${index}`} data-kind={item.kind}>
              <Link to={item.kind === 'skill' ? '/skills' : workLink(item)}>
                <span className="feed__line">
                  <Icon aria-hidden="true" />
                  <strong>
                    {named.get(item.agent)?.name ?? 'An agent'}
                  </strong>{' '}
                  {verb}
                </span>
                {item.text && (
                  <span className="feed__text">{preview(item.text)}</span>
                )}
              </Link>
              <time dateTime={item.at}>{when(item.at)}</time>
            </li>
          )
        })}
      </ul>
    </section>
  )
}

/** What the orchestrator says about itself; readiness covers the database, nothing more. */
function Platform({ data }: { data: Data }) {
  const installation = useInstallation()
  const ready = useQuery({
    queryKey: ['ready'],
    queryFn: () => admin.ready().then((response) => response.ok),
    refetchInterval: 15_000,
  })
  return (
    <section className="platform-strip" aria-label="Platform">
      <dl className="facts">
        <dt>Database</dt>
        <dd data-bad={ready.data === false}>
          {ready.data === undefined
            ? 'Checking…'
            : ready.data
              ? 'Ready'
              : 'Not ready'}
        </dd>
        <dt>Sandbox</dt>
        <dd>{installation.data ? sandboxLabel(installation.data) : '…'}</dd>
        {data.model && (
          <>
            <dt>Model</dt>
            <dd className="facts__wrap">{data.model}</dd>
          </>
        )}
        {data.people > 0 && (
          <>
            <dt>People</dt>
            <dd>{data.people}</dd>
            <dt>Rooms</dt>
            <dd>{data.rooms}</dd>
            <dt>Files stored</dt>
            <dd>{fileSize(data.stored_bytes)}</dd>
          </>
        )}
      </dl>
    </section>
  )
}
