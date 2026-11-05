import { Plus, X } from 'lucide-react'
import { useState } from 'react'
import { api, failure, ok } from '../../api/client'
import { useAgents } from '../../app/queries'
import { Alert, Field } from '../../components/primitives'
import { AccessChoice } from '../vault/InstallationVault'
import type { McpServer, SaveMcpServer } from './McpServers'

type Transport = SaveMcpServer['transport']
type Access = McpServer['access']
type Pair = { name: string; value: string }

export function McpForm({
  existing,
  onDone,
}: {
  existing: McpServer | undefined
  onDone: () => void
}) {
  const agents = useAgents()
  const [transport, setTransport] = useState<Transport>(
    existing?.transport ?? 'stdio',
  )
  const [access, setAccess] = useState<Access>(
    existing?.access ?? { mode: 'all_agents' },
  )
  const [replacing, setReplacing] = useState(!existing)
  const [pairs, setPairs] = useState<Pair[]>([])
  const [error, setError] = useState('')
  const [busy, setBusy] = useState(false)
  const mustReplace = Boolean(existing && existing.transport !== transport)
  async function save(form: HTMLFormElement) {
    const data = new FormData(form)
    const text = (name: string) => String(data.get(name) ?? '').trim()
    const body = {
      name: text('name'),
      transport,
      ...(transport === 'stdio'
        ? { command: text('command'), args: lines(text('args')) }
        : { url: text('url') }),
      secrets:
        replacing || mustReplace
          ? Object.fromEntries(
              pairs
                .filter((pair) => pair.name.trim())
                .map((pair) => [pair.name.trim(), pair.value]),
            )
          : null,
      access,
    }
    setBusy(true)
    setError('')
    try {
      await (existing
        ? ok(
            api.PUT('/api/mcp-servers/{server}', {
              params: { path: { server: existing.id } },
              body,
            }),
          )
        : ok(api.POST('/api/mcp-servers', { body })))
      onDone()
    } catch (error) {
      setError(failure(error))
    } finally {
      setBusy(false)
    }
  }
  return (
    <form
      className="form"
      onSubmit={(event) => {
        event.preventDefault()
        void save(event.currentTarget)
      }}
    >
      <Field
        label="Name"
        hint="Letters, digits and hyphens; prefixes its tools"
      >
        <input
          name="name"
          required
          maxLength={32}
          pattern="[A-Za-z0-9\-]{1,32}"
          placeholder="github"
          defaultValue={existing?.name ?? ''}
        />
      </Field>
      <TransportChoice value={transport} onPick={setTransport} />
      <TransportFields transport={transport} existing={existing} />
      <Secrets
        transport={transport}
        stored={mustReplace ? [] : (existing?.secret_names ?? [])}
        replacing={replacing || mustReplace}
        pairs={pairs}
        onReplace={() => {
          setReplacing(true)
          setPairs(
            (existing?.secret_names ?? []).map((name) => ({ name, value: '' })),
          )
        }}
        onChange={setPairs}
      />
      <AccessChoice
        legend="Which agents may use it"
        access={access}
        agents={(agents.data ?? []).filter(
          (agent) => agent.desired_state !== 'retired',
        )}
        onChange={setAccess}
      />
      <Alert>{error}</Alert>
      <div className="actions">
        <button className="btn btn--primary" disabled={busy}>
          {busy ? 'Saving…' : existing ? 'Save changes' : 'Add server'}
        </button>
        <button type="button" className="btn btn--ghost" onClick={onDone}>
          Cancel
        </button>
      </div>
    </form>
  )
}

function lines(text: string) {
  return text
    .split('\n')
    .map((line) => line.trim())
    .filter(Boolean)
}

function TransportChoice({
  value,
  onPick,
}: {
  value: Transport
  onPick: (transport: Transport) => void
}) {
  const options = [
    ['stdio', 'Local program'],
    ['http', 'Remote URL'],
  ] as const
  return (
    <div className="tabs" role="radiogroup" aria-label="Transport">
      {options.map(([transport, label]) => (
        <button
          key={transport}
          type="button"
          role="radio"
          aria-checked={value === transport}
          aria-selected={value === transport}
          onClick={() => onPick(transport)}
        >
          {label}
        </button>
      ))}
    </div>
  )
}

function TransportFields({
  transport,
  existing,
}: {
  transport: Transport
  existing: McpServer | undefined
}) {
  if (transport === 'http')
    return (
      <Field label="URL" hint="Streamable HTTP endpoint">
        <input
          name="url"
          type="url"
          required
          maxLength={2048}
          placeholder="https://mcp.example.com/mcp"
          defaultValue={existing?.url ?? ''}
        />
      </Field>
    )
  return (
    <>
      <Field label="Command" hint="Runs on each agent's computer as its user">
        <input
          name="command"
          required
          maxLength={4096}
          placeholder="npx"
          spellCheck={false}
          defaultValue={existing?.command ?? ''}
        />
      </Field>
      <Field label="Arguments" hint="One per line">
        <textarea
          name="args"
          rows={3}
          spellCheck={false}
          className="mono"
          placeholder={'-y\n@modelcontextprotocol/server-github'}
          defaultValue={existing?.args.join('\n') ?? ''}
        />
      </Field>
    </>
  )
}

function Secrets({
  transport,
  stored,
  replacing,
  pairs,
  onReplace,
  onChange,
}: {
  transport: Transport
  stored: string[]
  replacing: boolean
  pairs: Pair[]
  onReplace: () => void
  onChange: (pairs: Pair[]) => void
}) {
  const kind = transport === 'stdio' ? 'Environment variables' : 'Headers'
  const set = (index: number, pair: Pair) =>
    onChange(pairs.map((known, at) => (at === index ? pair : known)))
  if (!replacing)
    return (
      <div className="mcp-secrets">
        <span className="mcp-secrets__label">{kind}</span>
        <p className="small faint">
          {stored.length
            ? `${stored.join(', ')}: values stored encrypted.`
            : 'None stored.'}{' '}
          <button type="button" className="link-btn" onClick={onReplace}>
            Replace
          </button>
        </p>
      </div>
    )
  return (
    <fieldset className="mcp-secrets">
      <legend className="mcp-secrets__label">
        {kind}
        <span className="hint">Stored encrypted, never shown again</span>
      </legend>
      {pairs.map((pair, index) => (
        <div className="mcp-secrets__row" key={index}>
          <input
            aria-label={`${kind} name`}
            className="mono"
            spellCheck={false}
            placeholder={
              transport === 'stdio' ? 'GITHUB_TOKEN' : 'Authorization'
            }
            value={pair.name}
            onChange={(event) =>
              set(index, { ...pair, name: event.target.value })
            }
          />
          <input
            aria-label={`${pair.name || kind} value`}
            type="password"
            autoComplete="new-password"
            placeholder="Value"
            value={pair.value}
            onChange={(event) =>
              set(index, { ...pair, value: event.target.value })
            }
          />
          <button
            type="button"
            className="btn btn--ghost btn--icon btn--sm"
            aria-label={`Remove ${pair.name || 'this entry'}`}
            onClick={() => onChange(pairs.filter((_, at) => at !== index))}
          >
            <X />
          </button>
        </div>
      ))}
      <button
        type="button"
        className="btn btn--ghost btn--sm mcp-secrets__add"
        onClick={() => onChange([...pairs, { name: '', value: '' }])}
      >
        <Plus /> Add {transport === 'stdio' ? 'variable' : 'header'}
      </button>
    </fieldset>
  )
}
