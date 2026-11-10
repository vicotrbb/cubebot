import { useQuery, useQueryClient } from '@tanstack/react-query'
import { createFileRoute } from '@tanstack/react-router'
import { Minus, Plus } from 'lucide-react'
import { useState } from 'react'
import { api, failure, ok } from '../api/client'
import type { components } from '../api/schema'
import { CardSelect } from '../components/CardSelect'
import { Cube } from '../components/Cube'
import { Alert, Field, Skeleton, Status } from '../components/primitives'
import { PROVIDER_NAMES } from '../features/platform/models'
import { SaveBar, Section } from '../features/platform/Section'
import { ModelField, useModels } from '../features/platform/Providers'
import { ORCHESTRATOR_AVATAR } from '../features/tasks/orchestrator'
import '../features/platform/platform.css'
import '../features/routines/routines.css'

export const Route = createFileRoute('/_app/platform/tasks')({
  component: TaskSettingsPage,
})

type Settings = components['schemas']['TaskSettings']
type Provider = components['schemas']['ProviderKind']

const SAME = 'same'
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
      className="settings"
      onSubmit={(event) => {
        event.preventDefault()
        void save()
      }}
    >
      <Section
        title="What it thinks with"
        description={
          <div className="task-settings__hello">
            <Cube avatar={ORCHESTRATOR_AVATAR} size={30} />
            <p>
              The orchestrator takes every task people hand over: it names it,
              picks the agents, gives each their part and reads what comes back.
            </p>
          </div>
        }
      >
        <CardSelect
          label="What it thinks with"
          value={draft.provider ?? SAME}
          onChange={(choice) =>
            setDraft({
              ...draft,
              provider: choice === SAME ? null : choice,
              model: null,
            })
          }
          options={[
            {
              value: SAME,
              title: 'The same model as the agents',
              detail: active
                ? `${PROVIDER_NAMES[active]}, whichever model is in use there`
                : 'Set one up under AI providers first',
              mark: <Cube size={20} />,
            },
            ...keyed.map((provider) => ({
              value: provider,
              title: `A model from ${PROVIDER_NAMES[provider]}`,
              detail:
                draft.provider === provider && draft.model
                  ? draft.model
                  : 'A small, quick model is usually enough for this job',
              mark: PROVIDER_NAMES[provider].charAt(0),
            })),
          ]}
        />
        {draft.provider && (
          <ProviderModel
            key={draft.provider}
            provider={draft.provider}
            value={draft.model ?? ''}
            onPick={(model) => setDraft({ ...draft, model })}
          />
        )}
      </Section>
      <Section
        title="How far it goes"
        description="It stops and asks you once it reaches either limit."
      >
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
      </Section>
      <Section
        title="House rules"
        description="Optional. It follows these on every task, before any task's own words."
      >
        <Field label="Rules">
          <textarea
            rows={6}
            maxLength={8192}
            placeholder="Ayla handles anything about finance. Ask before spending money."
            value={draft.instructions}
            onChange={(event) =>
              setDraft({ ...draft, instructions: event.target.value })
            }
          />
        </Field>
      </Section>
      <SaveBar>
        <Alert>{error}</Alert>
        <Status>{notice}</Status>
        <div className="actions">
          <button className="btn btn--primary" disabled={busy || !ready}>
            {busy ? 'Saving…' : 'Save'}
          </button>
        </div>
      </SaveBar>
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
