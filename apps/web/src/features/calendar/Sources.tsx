import { Link } from '@tanstack/react-router'
import { Lock } from 'lucide-react'
import type { AgentSummary } from '../../api/types'
import { Cube, moodOf } from '../../components/Cube'
import { Mark } from '../connections/Mark'
import '../connections/connections.css'
import type { Source } from './parts'

type Toggles = {
  hidden: Set<string>
  onToggle: (id: string) => void
  paint: (source: string) => string
}

/** The calendars and agents on the calendar, each with its color and a show/hide box. */
export function Sources({
  sources,
  agents,
  administrator,
  ...toggles
}: Toggles & {
  sources: Source[]
  agents: AgentSummary[]
  administrator: boolean
}) {
  const accounts = new Map<string, Source[]>()
  for (const source of sources.filter((each) => each.kind === 'calendar')) {
    const key = source.connection ?? source.id
    accounts.set(key, [...(accounts.get(key) ?? []), source])
  }
  const crew = sources.filter((each) => each.kind === 'agent')
  return (
    <div className="cal-sources">
      <section aria-labelledby="cal-calendars">
        <h2 id="cal-calendars">Calendars</h2>
        {accounts.size === 0 && <NoCalendars administrator={administrator} />}
        {[...accounts.values()].map((group) => (
          <Account key={group[0]?.connection} group={group} {...toggles} />
        ))}
      </section>
      <section aria-labelledby="cal-agents">
        <h2 id="cal-agents">Agents</h2>
        {crew.length === 0 && (
          <p className="cal-sources__note">
            Routines of agents you create show here.
          </p>
        )}
        <ul>
          {crew.map((source) => {
            const agent = agents.find(
              (each) => `agent:${each.id}` === source.id,
            )
            return (
              <li key={source.id}>
                <Toggle source={source} {...toggles}>
                  <Cube
                    avatar={agent?.avatar ?? null}
                    mood={agent ? moodOf(agent) : 'idle'}
                    size={16}
                  />
                </Toggle>
              </li>
            )
          })}
        </ul>
      </section>
    </div>
  )
}

function NoCalendars({ administrator }: { administrator: boolean }) {
  if (!administrator)
    return (
      <p className="cal-sources__note">
        Your agents&rsquo; routines show here. Calendars come from connections,
        and an administrator adds those.
      </p>
    )
  return (
    <p className="cal-sources__note">
      Show your own events beside your agents&rsquo; work.{' '}
      <Link to="/platform/connections">Connect a calendar</Link>
    </p>
  )
}

function Account({ group, ...toggles }: Toggles & { group: Source[] }) {
  const head = group[0]
  if (!head) return null
  // A connection whose calendars could not be listed comes as one source without a calendar.
  const broken = group.find(
    (source) => source.id === `calendar:${source.connection}`,
  )
  const calendars = group.filter((source) => source !== broken)
  return (
    <div className="cal-account">
      <div className="cal-account__head">
        <Mark id={head.connector ?? ''} />
        <span className="cal-account__name">{head.account ?? head.name}</span>
      </div>
      {broken?.problem && <p className="cal-problem">{broken.problem}</p>}
      <ul>
        {calendars.map((source) => (
          <li key={source.id}>
            <Toggle source={source} {...toggles} />
            {source.problem && <p className="cal-problem">{source.problem}</p>}
          </li>
        ))}
      </ul>
    </div>
  )
}

function Toggle({
  source,
  hidden,
  onToggle,
  paint,
  children,
}: Toggles & { source: Source; children?: React.ReactNode }) {
  const readOnly = source.kind === 'calendar' && !source.writable
  return (
    <label className="cal-source">
      <input
        type="checkbox"
        checked={!hidden.has(source.id)}
        onChange={() => onToggle(source.id)}
        style={{ '--c': paint(source.id) } as React.CSSProperties}
      />
      {children}
      <span className="cal-source__name">{source.name}</span>
      {readOnly && (
        <span className="cal-source__lock" title="Read only">
          <Lock aria-hidden="true" />
          <span className="sr-only">Read only</span>
        </span>
      )}
    </label>
  )
}
