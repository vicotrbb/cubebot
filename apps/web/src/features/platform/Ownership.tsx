import { useQuery } from '@tanstack/react-query'
import { Search } from 'lucide-react'
import { useState } from 'react'
import { admin, type Owned } from '../../api/admin'
import { Alert, Empty, Skeleton, Status } from '../../components/primitives'
import { heirs } from './roles'
import { useAdminAction } from './useAdminAction'
import { useViewer } from './useViewer'
import './platform.css'

type Item = Awaited<ReturnType<typeof admin.inventory>>[number]
type Account = Awaited<ReturnType<typeof admin.accounts>>[number]
type Action = ReturnType<typeof useAdminAction>

const KINDS = [
  { id: '', label: 'All' },
  { id: 'agent', label: 'Agents' },
  { id: 'room', label: 'Rooms' },
  { id: 'team', label: 'Teams' },
  { id: 'connection', label: 'Connections' },
] as const

const KIND: Record<string, string> = {
  agent: 'Agent',
  room: 'Room',
  team: 'Team',
  connection: 'Connection',
}

export function PlatformOwnership() {
  const inventory = useQuery({
    queryKey: ['inventory'],
    queryFn: admin.inventory,
  })
  const accounts = useQuery({ queryKey: ['accounts'], queryFn: admin.accounts })
  const viewer = useViewer()
  const action = useAdminAction(['inventory', 'accounts'])
  const items = inventory.data ?? []
  const orphans = items.filter((item) => !item.owner_active)
  const give = viewer.writer ? (accounts.data ?? []) : null
  return (
    <div className="ownership">
      <p className="platform__lead">
        Every agent, room, team and connection, and the person in charge of it.
        An owner decides who else can reach what they own.
      </p>
      <Alert>{action.error}</Alert>
      <Status>{action.notice}</Status>
      {inventory.isPending && <Skeleton lines={6} />}
      {inventory.isError && <Alert>What people own could not be loaded.</Alert>}
      {orphans.length > 0 && (
        <section className="ownership__section">
          <h2 className="platform__h2">Needs a new owner</h2>
          <p className="platform__lead">
            Their owner is suspended. Give each to someone who stays, or give
            everything at once from People.
          </p>
          <Inventory items={orphans} give={give} action={action} />
        </section>
      )}
      {inventory.data && <AllItems items={items} give={give} action={action} />}
      <SpendByPerson />
    </div>
  )
}

function AllItems({
  items,
  give,
  action,
}: {
  items: Item[]
  give: Account[] | null
  action: Action
}) {
  const [kind, setKind] = useState('')
  const [owner, setOwner] = useState('')
  const [search, setSearch] = useState('')
  const owners = [
    ...new Map(items.map((item) => [item.owner, item.owner_login])),
  ].sort((a, b) => a[1].localeCompare(b[1]))
  const term = search.trim().toLowerCase()
  const shown = items.filter(
    (item) =>
      (!kind || item.kind === kind) &&
      (!owner || item.owner === owner) &&
      item.name.toLowerCase().includes(term),
  )
  return (
    <section className="ownership__section">
      <h2 className="platform__h2">Everything</h2>
      <div className="ownership__filters">
        <nav className="tabs" aria-label="Filter by kind">
          {KINDS.map((item) => (
            <button
              key={item.id}
              type="button"
              aria-pressed={kind === item.id}
              onClick={() => setKind(item.id)}
            >
              {item.label}
            </button>
          ))}
        </nav>
        <select
          className="input ownership__owner"
          aria-label="Filter by owner"
          value={owner}
          onChange={(event) => setOwner(event.target.value)}
        >
          <option value="">Anyone</option>
          {owners.map(([id, login]) => (
            <option key={id} value={id}>
              {login}
            </option>
          ))}
        </select>
        <label className="access__search">
          <Search aria-hidden="true" />
          <input
            type="search"
            aria-label="Filter by name"
            placeholder="Filter"
            value={search}
            onChange={(event) => setSearch(event.target.value)}
          />
        </label>
      </div>
      {items.length === 0 ? (
        <Empty title="Nobody owns anything yet.">
          Agents, rooms, teams and personal connections appear here as people
          create them.
        </Empty>
      ) : (
        <Inventory items={shown} give={give} action={action} />
      )}
    </section>
  )
}

function Inventory({
  items,
  give,
  action,
}: {
  items: Item[]
  give: Account[] | null
  action: Action
}) {
  return (
    <table className="table ownership__table">
      <thead>
        <tr>
          <th>Name</th>
          <th>Kind</th>
          <th>Owner</th>
          <th>Shared</th>
          {give && <th>Give to</th>}
        </tr>
      </thead>
      <tbody>
        {items.length === 0 && (
          <tr>
            <td colSpan={give ? 5 : 4} className="faint">
              Nothing matches that.
            </td>
          </tr>
        )}
        {items.map((item) => (
          <tr key={`${item.kind}:${item.id}`}>
            <td>{item.name}</td>
            <td>{KIND[item.kind] ?? item.kind}</td>
            <td>
              {item.owner_login}
              {!item.owner_active && (
                <span className="chip chip--danger ownership__chip">
                  Suspended
                </span>
              )}
            </td>
            <td className="tabular">{shared(item)}</td>
            {give && (
              <td>
                <GiveTo item={item} people={give} action={action} />
              </td>
            )}
          </tr>
        ))}
      </tbody>
    </table>
  )
}

function shared(item: Item) {
  if (item.kind === 'connection' && !item.personal) return 'Installation'
  if (item.shares === 0) return <span className="faint">Not shared</span>
  return `${item.shares} ${item.shares === 1 ? 'grant' : 'grants'}`
}

function GiveTo({
  item,
  people,
  action,
}: {
  item: Item
  people: Account[]
  action: Action
}) {
  if (item.kind === 'connection')
    return (
      <span
        className="small faint"
        title="A personal connection holds its owner’s own credentials, so it cannot change hands."
      >
        Stays with its owner
      </span>
    )
  const choices = heirs(people, item.owner)
  return (
    <select
      className="input ownership__give"
      aria-label={`Give ${item.name} to`}
      value=""
      disabled={action.busy || choices.length === 0}
      onChange={(event) => {
        const to = choices.find((person) => person.id === event.target.value)
        if (to)
          void action.run(
            () => admin.transfer(item.kind as Owned, item.id, to.id),
            `${to.login} owns ${item.name} now.`,
          )
      }}
    >
      <option value="" disabled>
        Give to…
      </option>
      {choices.map((person) => (
        <option key={person.id} value={person.id}>
          {person.login}
        </option>
      ))}
    </select>
  )
}

const PERIODS = [7, 30, 90] as const
const money = new Intl.NumberFormat(undefined, {
  style: 'currency',
  currency: 'USD',
})

function SpendByPerson() {
  const [days, setDays] = useState<number>(30)
  const spend = useQuery({
    queryKey: ['spend', days],
    queryFn: () => admin.spend(days),
  })
  return (
    <section className="ownership__section">
      <div className="ownership__filters">
        <h2 className="platform__h2">Spend by person</h2>
        <div className="tabs" role="group" aria-label="Period">
          {PERIODS.map((period) => (
            <button
              key={period}
              type="button"
              aria-pressed={days === period}
              onClick={() => setDays(period)}
            >
              {period} days
            </button>
          ))}
        </div>
      </div>
      <p className="platform__lead">
        Model use by the agents each person owns now. Cost counts only models
        with a known price.
      </p>
      {spend.isPending && <Skeleton lines={3} />}
      {spend.isError && <Alert>Spend could not be loaded.</Alert>}
      {spend.data && (
        <table className="table">
          <thead>
            <tr>
              <th>Person</th>
              <th className="num">Agents</th>
              <th className="num">Input tokens</th>
              <th className="num">Output tokens</th>
              <th className="num">Cost</th>
            </tr>
          </thead>
          <tbody>
            {spend.data.length === 0 && (
              <tr>
                <td colSpan={5} className="faint">
                  No model use in the last {days} days.
                </td>
              </tr>
            )}
            {spend.data.map((row) => (
              <tr key={row.account}>
                <td>{row.login}</td>
                <td className="num">{row.agents.toLocaleString()}</td>
                <td className="num">{row.input_tokens.toLocaleString()}</td>
                <td className="num">{row.output_tokens.toLocaleString()}</td>
                <td className="num">
                  {row.cost_usd == null ? (
                    <span className="faint">No price known</span>
                  ) : (
                    money.format(row.cost_usd)
                  )}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </section>
  )
}
