import { useQuery, useQueryClient } from '@tanstack/react-query'
import { useState } from 'react'
import { api, failure, ok } from '../../api/client'
import { can, type AgentSummary, type Session } from '../../api/types'
import { Alert, Field, Skeleton, Status } from '../../components/primitives'
import { DayBars } from './DayBars'
import { count, money } from './format'
import { BudgetMeter, modelCost } from './Situation'
import './situation.css'

/** What this agent spends, and the limit that keeps it honest. */
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
        <h2>This month</h2>
        <BudgetMeter
          spent={data.month.cost_usd}
          budget={data.monthly_budget_usd}
        />
        {data.paused_turns > 0 && (
          <p className="notice notice--danger small" role="status">
            {data.paused_turns}{' '}
            {data.paused_turns === 1 ? 'turn is' : 'turns are'} paused by this
            budget. Raising it, or a new month, resumes them.
          </p>
        )}
        {can(agent, 'agent_manage', session) && (
          <BudgetForm
            key={data.monthly_budget_usd ?? 'none'}
            agent={agent.id}
            budget={data.monthly_budget_usd ?? null}
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

function BudgetForm({
  agent,
  budget,
}: {
  agent: string
  budget: number | null
}) {
  const client = useQueryClient()
  const [value, setValue] = useState(budget === null ? '' : String(budget))
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  const [notice, setNotice] = useState('')
  async function save(next: number | null) {
    setBusy(true)
    setError('')
    setNotice('')
    try {
      const saved = await ok(
        api.PUT('/api/agents/{agent}/budget', {
          params: { path: { agent } },
          body: { monthly_budget_usd: next },
        }),
      )
      client.setQueryData(['agent-usage', agent], saved)
      setNotice(next === null ? 'Limit removed.' : 'Budget saved.')
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
        const next = value.trim() === '' ? null : Number(value)
        if (next !== null && !(next > 0)) {
          setError(
            'Enter an amount above zero, or leave it empty for no limit.',
          )
          return
        }
        void save(next)
      }}
    >
      <div className="usage__budget">
        <Field label="Monthly budget (USD)">
          <input
            inputMode="decimal"
            value={value}
            placeholder="No limit"
            onChange={(event) => setValue(event.target.value)}
          />
        </Field>
        <button className="btn btn--sm" disabled={busy}>
          {busy ? 'Saving…' : 'Save'}
        </button>
      </div>
      <p className="board-section__note">
        Past it, the agent pauses its work until you raise this. Empty means no
        limit.
      </p>
      <Alert>{error}</Alert>
      <Status>{notice}</Status>
    </form>
  )
}
