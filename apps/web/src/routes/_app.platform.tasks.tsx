import { useQuery, useQueryClient } from '@tanstack/react-query'
import { createFileRoute } from '@tanstack/react-router'
import { Minus, Plus } from 'lucide-react'
import { useState } from 'react'
import { api, failure, ok } from '../api/client'
import type { components } from '../api/schema'
import { Cube } from '../components/Cube'
import { Alert, Field, Skeleton, Status } from '../components/primitives'
import { ModelField, useModels } from '../features/platform/Providers'
import { ORCHESTRATOR_AVATAR } from '../features/tasks/orchestrator'
import '../features/platform/platform.css'
import '../features/routines/routines.css'

export const Route = createFileRoute('/_app/platform/tasks')({
  component: TaskSettingsPage,
})

type Settings = components['schemas']['TaskSettings']
type Provider = components['schemas']['ProviderKind']

const NAMES: Record<Provider, string> = {
  openai: 'OpenAI',
  anthropic: 'Anthropic',
  openrouter: 'OpenRouter',
  gemini: 'Google Gemini',
  grok: 'xAI Grok',
  chatgpt: 'ChatGPT plan',
  supergrok: 'Grok plan',
  minimax: 'MiniMax',
  minimax_cn: 'MiniMax (mainland China)',
  minimax_plan: 'MiniMax M Plan',
  minimax_plan_cn: 'MiniMax M Plan (mainland China)',
}
const PLANS: Provider[] = [
  'chatgpt',
  'supergrok',
  'minimax_plan',
  'minimax_plan_cn',
]
/** A plan is personal, so it never becomes the installation's task orchestrator by choice. */
const keyed = (provider: Provider) => !PLANS.includes(provider)

function TaskSettingsPage() {
  const settings = useQuery({
    queryKey: ['task-settings'],
    queryFn: () => ok(api.GET('/api/tasks/settings')),
  })
  if (settings.isPending) return <Skeleton lines={4} width="62ch" />
  if (!settings.data)
    return <Alert>The settings could not be read. Retry shortly.</Alert>
  return <Form saved={settings.data} />
}

function useKeyedProviders() {
  const current = useQuery({
    queryKey: ['provider'],
    queryFn: () => ok(api.GET('/api/provider')),
  })
  const saved = useQuery({
    queryKey: ['provider-saved'],
    queryFn: () => ok(api.GET('/api/provider/saved')),
  })
  const active = current.data?.provider
  return {
    active,
    keyed: [
      ...(active ? [active] : []),
      ...(saved.data ?? [])
        .map((item) => item.provider)
        .filter((provider) => provider !== active),
    ].filter(keyed),
  }
}

function Form({ saved }: { saved: Settings }) {
  const client = useQueryClient()
  const { active, keyed } = useKeyedProviders()
  const [draft, setDraft] = useState(saved)
  const [error, setError] = useState('')
  const [notice, setNotice] = useState('')
  const [busy, setBusy] = useState(false)
  const own = draft.provider != null
  const save = async () => {
    setBusy(true)
    setError('')
    setNotice('')
    try {
      const stored = await ok(api.PUT('/api/tasks/settings', { body: draft }))
      setDraft(stored)
      client.setQueryData(['task-settings'], stored)
      setNotice('Saved. The orchestrator uses this from its next decision.')
    } catch (error) {
      setError(failure(error))
    } finally {
      setBusy(false)
    }
  }
  const ready = !own || Boolean(draft.model)
  return (
    <form
      className="form task-settings"
      onSubmit={(event) => {
        event.preventDefault()
        void save()
      }}
    >
      <header className="task-settings__hello">
        <Cube avatar={ORCHESTRATOR_AVATAR} size={44} />
        <p>
          The orchestrator takes every task people hand over: it names it, picks
          the agents, gives each their part and reads what comes back.
        </p>
      </header>
      <fieldset className="task-settings__model">
        <legend>What it thinks with</legend>
        <label className="trigger">
          <input
            type="radio"
            name="source"
            checked={!own}
            onChange={() => setDraft({ ...draft, provider: null, model: null })}
          />
          <span>
            The same model as the agents
            <span className="small faint">
              {active
                ? `${NAMES[active]}, whichever model is in use there`
                : 'Set one up under AI providers first'}
            </span>
          </span>
        </label>
        {keyed.map((provider) => (
          <label className="trigger" key={provider}>
            <input
              type="radio"
              name="source"
              checked={draft.provider === provider}
              onChange={() => setDraft({ ...draft, provider, model: null })}
            />
            <span>
              A model from {NAMES[provider]}
              <span className="small faint">
                A small, quick model is usually enough for this job
              </span>
            </span>
          </label>
        ))}
        {draft.provider && (
          <ProviderModel
            key={draft.provider}
            provider={draft.provider}
            value={draft.model ?? ''}
            onPick={(model) => setDraft({ ...draft, model })}
          />
        )}
      </fieldset>
      <Stepper
        label="Agents per round"
        hint="The most it may start at once"
        value={draft.max_agents}
        onChange={(max_agents) => setDraft({ ...draft, max_agents })}
      />
      <Stepper
        label="Rounds of work"
        hint="How many times it may start agents before it asks you"
        value={draft.max_rounds}
        onChange={(max_rounds) => setDraft({ ...draft, max_rounds })}
      />
      <Field
        label="House rules"
        hint="Optional; it follows these on every task"
      >
        <textarea
          rows={4}
          maxLength={8192}
          placeholder="Ayla handles anything about finance. Ask before spending money."
          value={draft.instructions}
          onChange={(event) =>
            setDraft({ ...draft, instructions: event.target.value })
          }
        />
      </Field>
      <Alert>{error}</Alert>
      <Status>{notice}</Status>
      <div className="actions">
        <button className="btn btn--primary" disabled={busy || !ready}>
          {busy ? 'Saving…' : 'Save'}
        </button>
      </div>
    </form>
  )
}

function ProviderModel({
  provider,
  value,
  onPick,
}: {
  provider: Provider
  value: string
  onPick: (model: string) => void
}) {
  const list = useModels(provider, '', true)
  return (
    <div className="task-settings__picker">
      {list.problem && (
        <p role="alert" className="small provider-problem">
          {list.problem}
        </p>
      )}
      <ModelField
        models={list.models}
        typed={list.unreachable}
        value={value}
        onPick={onPick}
      />
    </div>
  )
}

function Stepper({
  label,
  hint,
  value,
  onChange,
}: {
  label: string
  hint: string
  value: number
  onChange: (value: number) => void
}) {
  return (
    <div className="stepper">
      <span>
        {label}
        <span className="small faint">{hint}</span>
      </span>
      <span className="stepper__controls">
        <button
          type="button"
          className="btn btn--icon"
          aria-label={`Fewer: ${label}`}
          disabled={value <= 1}
          onClick={() => onChange(value - 1)}
        >
          <Minus />
        </button>
        <output className="tabular" aria-label={label}>
          {value}
        </output>
        <button
          type="button"
          className="btn btn--icon"
          aria-label={`More: ${label}`}
          disabled={value >= 16}
          onClick={() => onChange(value + 1)}
        >
          <Plus />
        </button>
      </span>
    </div>
  )
}
