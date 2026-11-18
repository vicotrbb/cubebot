import { ArrowDown, ArrowUp } from 'lucide-react'
import type { CSSProperties, ReactNode } from 'react'
import type { AgentSummary } from '../../api/types'
import { Alert, Skeleton } from '../../components/primitives'
import {
  BUSY,
  STALE_SECONDS,
  age,
  bytes,
  percent,
  rate,
  share,
  useVitals,
  type Reading,
  type Usage,
} from './reading'
import './vitals.css'

type Vital = {
  label: string
  value: ReactNode
  detail?: string
  load?: number
}

function processor(reading: Reading): Vital {
  const cpus = `${reading.cpus} ${reading.cpus === 1 ? 'CPU' : 'CPUs'}`
  if (reading.cpu_percent == null)
    return { label: 'CPU', value: 'Measuring…', detail: cpus }
  const load = reading.cpu_percent / 100
  return { label: 'CPU', value: percent(load), detail: cpus, load }
}

function capacity(label: string, usage: Usage | null | undefined): Vital {
  if (!usage) return { label, value: 'Not measured' }
  return {
    label,
    value: bytes(usage.used),
    detail: `of ${bytes(usage.total)}`,
    load: share(usage),
  }
}

function traffic(reading: Reading): Vital {
  const network = reading.network
  if (!network) return { label: 'Network', value: 'Measuring…' }
  return {
    label: 'Network',
    value: (
      <span className="vitals__traffic">
        <span aria-label={`${rate(network.received)} in`}>
          <ArrowDown aria-hidden="true" />
          {rate(network.received)}
        </span>
        <span aria-label={`${rate(network.sent)} out`}>
          <ArrowUp aria-hidden="true" />
          {rate(network.sent)}
        </span>
      </span>
    ),
  }
}

const vitalsOf = (reading: Reading) => [
  processor(reading),
  capacity('Memory', reading.memory),
  capacity('Storage', reading.disk),
  traffic(reading),
  ...(reading.apps_running == null
    ? []
    : [{ label: 'Apps', value: `${reading.apps_running} running` }]),
]

function Meter({ load }: { load: number }) {
  return (
    <span
      className={load >= BUSY ? 'vitals__meter busy' : 'vitals__meter'}
      style={{ '--load': load } as CSSProperties}
      aria-hidden="true"
    />
  )
}

function Rows({ reading }: { reading: Reading }) {
  return (
    <dl className="vitals__rows tabular">
      {vitalsOf(reading).map((vital) => (
        <div
          key={vital.label}
          className={
            vital.load === undefined ? 'vitals__row bare' : 'vitals__row'
          }
        >
          <dt>{vital.label}</dt>
          <dd>
            <span className="vitals__value">{vital.value}</span>
            {vital.detail && (
              <span className="vitals__detail"> {vital.detail}</span>
            )}
          </dd>
          {vital.load !== undefined && <Meter load={vital.load} />}
        </div>
      ))}
    </dl>
  )
}

function Reported({ reading }: { reading: Reading }) {
  const stale = reading.seconds_ago > STALE_SECONDS
  return (
    <div className={stale ? 'vitals stale' : 'vitals'}>
      <Rows reading={reading} />
      {stale && (
        <p className="faint small">
          No reading for {age(reading.seconds_ago).replace(' ago', '')}. The
          agent may be restarting, or its connection dropped.
        </p>
      )}
    </div>
  )
}

export function VitalsSection({ agent }: { agent: AgentSummary }) {
  const running = agent.desired_state === 'running'
  const vitals = useVitals(agent.id, running)
  const reading = vitals.data
  return (
    <section className="panel__section" aria-labelledby="vitals-head">
      <div className="vitals__head">
        <h2 id="vitals-head">Computer</h2>
        {reading && (
          <span className="faint small">{age(reading.seconds_ago)}</span>
        )}
      </div>
      {!running && (
        <p className="faint small">
          The computer is off. Start the agent to see how it is running.
        </p>
      )}
      {running && vitals.isPending && <Skeleton lines={4} />}
      {running && vitals.isError && (
        <Alert>The computer's vitals could not be loaded.</Alert>
      )}
      {running && reading === null && (
        <p className="faint small">
          No reading yet. The agent reports every few seconds once its computer
          is up.
        </p>
      )}
      {running && reading && <Reported reading={reading} />}
    </section>
  )
}

export function VitalsOverlay({ agent }: { agent: AgentSummary }) {
  const vitals = useVitals(agent.id, agent.desired_state === 'running')
  const reading = vitals.data
  if (!reading) return null
  return (
    <div
      className={
        reading.seconds_ago > STALE_SECONDS ? 'vitals-hud stale' : 'vitals-hud'
      }
      role="group"
      aria-label={`${agent.name}'s computer vitals`}
    >
      <Rows reading={reading} />
    </div>
  )
}
