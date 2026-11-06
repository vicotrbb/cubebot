import { useQuery, useQueryClient } from '@tanstack/react-query'
import { useState } from 'react'
import { api, failure, ok } from '../../api/client'
import { can, type AgentSummary, type Session } from '../../api/types'
import { Alert, Field, Skeleton, Status } from '../../components/primitives'
import { DayBars } from './DayBars'
import { count, money } from './format'
import { BudgetMeter, modelCost } from './Situation'
import './situation.css'

export function UsageTab({
  agent,
  session,
}: {
  agent: AgentSummary
  session: Session
}) {
  const usage = useQuery({
    queryKey: ['agent-usage', agent.id],
    queryFn: () =>
      ok(
        api.GET('/api/agents/{agent}/usage', {
          params: { path: { agent: agent.id } },
        }),
      ),
    refetchInterval: 15_000,
  })
  if (usage.isError)
    return (
      <section className="panel__section">
        <Alert>{failure(usage.error)}</Alert>
      </section>
    )
  if (!usage.data)
    return (
      <section className="panel__section">
        <Skeleton lines={5} />
      </section>
    )
  const data = usage.data
  const rate =
    data.turns > 0 ? Math.round((data.failed_turns / data.turns) * 100) : 0
  return (
    <>
      <section className="panel__section usage">
        <h2>Budgets</h2>
        <div className="usage__limits">
          <div className="usage__limit">
            <span className="usage__period">Today</span>
            <BudgetMeter
              spent={data.today.cost_usd}
              budget={data.daily_budget_usd}
              period="daily"
            />
          </div>
          <div className="usage__limit">
            <span className="usage__period">This month</span>
            <BudgetMeter
              spent={data.month.cost_usd}
              budget={data.monthly_budget_usd}
            />
          </div>
        </div>
        {data.paused_turns > 0 && (
          <p className="notice notice--danger small" role="status">
            {data.paused_turns}{' '}
            {data.paused_turns === 1 ? 'turn is' : 'turns are'} paused by a
            budget. Raising it, or a new day or month, resumes them.
          </p>
        )}
        {can(agent, 'agent_manage', session) && (
          <BudgetForm
            agent={agent.id}
            daily={data.daily_budget_usd ?? null}
            monthly={data.monthly_budget_usd ?? null}
          />
        )}
      </section>
      <section className="panel__section usage">
        <h2>Last 30 days</h2>
        <dl className="kv">
          <dt>Today</dt>
          <dd>
            {money(data.today.cost_usd)} ·{' '}
            {count(data.today.input_tokens + data.today.output_tokens)} tokens
          </dd>
          <dt>7 days</dt>
          <dd>
            {money(data.week.cost_usd)} ·{' '}
            {count(data.week.input_tokens + data.week.output_tokens)} tokens
          </dd>
          <dt>Turns</dt>
          <dd>
            {data.turns}
            {data.failed_turns > 0 &&
              ` · ${data.failed_turns} failed or waiting (${rate}%)`}
          </dd>
        </dl>
        <DayBars
          days={data.days}
          measure="cost_usd"
          format={money}
          label="Estimated cost"
        />
        {data.models.length > 0 && (
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
        )}
        <p className="board-section__note">
          Costs are estimates from each model&rsquo;s listed price, by UTC day.
          {data.month.unpriced > 0 &&
            ` ${data.month.unpriced} ${data.month.unpriced === 1 ? 'response' : 'responses'} this month had no price and count only as tokens.`}
        </p>
      </section>
    </>
  )
}

const shown = (budget: number | null) => (budget === null ? '' : String(budget))
/** `undefined` when the entry is neither empty nor an amount above zero. */
const amount = (entry: string) => {
  if (entry.trim() === '') return null
  const value = Number(entry)
  return value > 0 && Number.isFinite(value) ? value : undefined
}

function BudgetForm({
  agent,
  daily,
  monthly,
}: {
  agent: string
  daily: number | null
  monthly: number | null
}) {
  const client = useQueryClient()
  const [day, setDay] = useState(shown(daily))
  const [month, setMonth] = useState(shown(monthly))
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  const [notice, setNotice] = useState('')
  const changed = day !== shown(daily) || month !== shown(monthly)
  async function save(body: {
    daily_budget_usd: number | null
    monthly_budget_usd: number | null
  }) {
    setBusy(true)
    setError('')
    setNotice('')
    try {
      const saved = await ok(
        api.PUT('/api/agents/{agent}/budget', {
          params: { path: { agent } },
          body,
        }),
      )
      client.setQueryData(['agent-usage', agent], saved)
      setNotice('Budgets saved.')
    } catch (error) {
      setError(failure(error))
    } finally {
      setBusy(false)
    }
  }
  return (
    <form
      className="form usage__budgetform"
      onSubmit={(event) => {
        event.preventDefault()
        const daily_budget_usd = amount(day)
        const monthly_budget_usd = amount(month)
        if (daily_budget_usd === undefined || monthly_budget_usd === undefined)
          return setError(
            'Enter an amount above zero, or leave it empty for no limit.',
          )
        if (
          daily_budget_usd !== null &&
          monthly_budget_usd !== null &&
          daily_budget_usd > monthly_budget_usd
        )
          return setError('A day cannot allow more than the whole month.')
        void save({ daily_budget_usd, monthly_budget_usd })
      }}
    >
      <div className="usage__budget">
        <Field label="Per day (USD)">
          <input
            inputMode="decimal"
            value={day}
            placeholder="No limit"
            onChange={(event) => setDay(event.target.value)}
          />
        </Field>
        <Field label="Per month (USD)">
          <input
            inputMode="decimal"
            value={month}
            placeholder="No limit"
            onChange={(event) => setMonth(event.target.value)}
          />
        </Field>
      </div>
      <p className="board-section__note">
        Past either, the agent pauses its work until the next UTC day or month,
        or until you raise it. Empty means no limit.
      </p>
      <Alert>{error}</Alert>
      <Status>{notice}</Status>
      <div className="actions">
        <button className="btn btn--sm" disabled={busy || !changed}>
          {busy ? 'Saving…' : 'Save budgets'}
        </button>
      </div>
    </form>
  )
}
