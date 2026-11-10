import { useQuery, useQueryClient } from '@tanstack/react-query'
import { useState } from 'react'
import { api, failure, ok } from '../../api/client'
import type { components } from '../../api/schema'
import type { AgentSummary } from '../../api/types'
import { CardSelect, type CardOption } from '../../components/CardSelect'
import { Cube } from '../../components/Cube'
import { Alert, Skeleton, Status } from '../../components/primitives'
import { PROVIDER_NAMES, suggestedWindow } from '../platform/models'
import { ModelField } from '../platform/Providers'
import '../platform/platform.css'
import { DEFAULT, differs, draftFor, ready, type Draft } from './model'

type Provider = components['schemas']['ProviderKind']
type Choice = components['schemas']['ModelChoice']
type Described = components['schemas']['AgentModel']
type Chosen = components['schemas']['ModelRef']

export function AgentModel({ agent }: { agent: AgentSummary }) {
  const described = useQuery({
    queryKey: ['agent-model', agent.id],
    queryFn: () =>
      ok(
        api.GET('/api/agents/{agent}/model', {
          params: { path: { agent: agent.id } },
        }),
      ),
  })
  return (
    <section className="panel__section agent-model">
      <h2>Model</h2>
      {described.data ? (
        <ModelForm
          key={JSON.stringify(described.data.chosen)}
          agent={agent}
          described={described.data}
        />
      ) : described.isError ? (
        <Alert>{failure(described.error)}</Alert>
      ) : (
        <Skeleton lines={2} />
      )}
    </section>
  )
}

function ModelForm({
  agent,
  described,
}: {
  agent: AgentSummary
  described: Described
}) {
  const client = useQueryClient()
  const saved = described.chosen ?? null
  const [draft, setDraft] = useState(() =>
    saved ? draftFor(saved.provider, saved) : null,
  )
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  const [notice, setNotice] = useState('')
  const save = async () => {
    setBusy(true)
    setError('')
    setNotice('')
    try {
      const stored = await ok(
        api.PUT('/api/agents/{agent}/model', {
          params: { path: { agent: agent.id } },
          body: {
            chosen: draft && {
              provider: draft.provider,
              model: draft.model,
              context_window: draft.window,
            },
          },
        }),
      )
      client.setQueryData(['agent-model', agent.id], stored)
      setNotice(`Saved. ${agent.name} uses it from its next turn.`)
    } catch (caught) {
      setError(failure(caught))
    } finally {
      setBusy(false)
    }
  }
  return (
    <form
      className="form"
      onSubmit={(event) => {
        event.preventDefault()
        void save()
      }}
    >
      <p className="panel__lead">
        What {agent.name} thinks with. The platform default follows AI
        providers; a model of its own stays until you change it here.
      </p>
      <CardSelect
        label={`What ${agent.name} thinks with`}
        value={draft?.provider ?? DEFAULT}
        onChange={(choice) => setDraft(draftFor(choice, saved))}
        options={choices(agent, described, draft)}
      />
      <Lost agent={agent} saved={saved} providers={described.providers} />
      {draft && (
        <Models
          key={draft.provider}
          agent={agent.id}
          provider={draft.provider}
          value={draft.model}
          onPick={(model, reported) =>
            setDraft({
              ...draft,
              model,
              window: suggestedWindow(model, reported)?.tokens ?? null,
            })
          }
        />
      )}
      <Alert>{error}</Alert>
      <Status>{notice}</Status>
      <div className="actions">
        <button
          className="btn btn--sm"
          disabled={busy || !differs(draft, saved) || !ready(draft)}
        >
          {busy ? 'Saving…' : 'Save model'}
        </button>
      </div>
    </form>
  )
}

function choices(
  agent: AgentSummary,
  described: Described,
  draft: Draft | null,
): CardOption<Provider | typeof DEFAULT>[] {
  const fallback = described.default
  return [
    {
      value: DEFAULT,
      title: 'The platform default',
      detail: fallback
        ? `${fallback.model} on ${PROVIDER_NAMES[fallback.provider]}`
        : 'Set one up under AI providers first',
      mark: <Cube avatar={agent.avatar} size={20} />,
    },
    ...described.providers.map((provider) => ({
      value: provider,
      title: `A model from ${PROVIDER_NAMES[provider]}`,
      detail:
        draft?.provider === provider && draft.model
          ? draft.model
          : 'Pick from the models its key can use',
      mark: PROVIDER_NAMES[provider].charAt(0),
    })),
  ]
}

function Lost({
  agent,
  saved,
  providers,
}: {
  agent: AgentSummary
  saved: Chosen | null
  providers: Provider[]
}) {
  if (!saved || providers.includes(saved.provider)) return null
  return (
    <p className="notice small" role="status">
      The key for {PROVIDER_NAMES[saved.provider]} was removed, so {agent.name}{' '}
      uses the platform default until you pick again.
    </p>
  )
}

function Models({
  agent,
  provider,
  value,
  onPick,
}: {
  agent: string
  provider: Provider
  value: string
  onPick: (model: string, reported: number | null) => void
}) {
  const listing = useQuery({
    queryKey: ['agent-models', agent, provider],
    retry: false,
    queryFn: async () => {
      const { data, response } = await api
        .POST('/api/agents/{agent}/models', {
          params: { path: { agent } },
          body: { provider },
        })
        .catch(() => ({ data: undefined, response: null }))
      if (!data) throw new Error(String(response?.status ?? 0))
      return data
    },
  })
  const status = listing.error ? Number(listing.error.message) : null
  const typed = status !== null
  const unreachable = status === 502 || status === 0
  const models: Choice[] | null = listing.data ?? null
  if (listing.isPending) return <Skeleton lines={3} />
  return (
    <>
      {status !== null && (
        <p role="alert" className="small provider-problem">
          {unreachable
            ? `${PROVIDER_NAMES[provider]} could not be reached. Type the model ID instead, or retry.`
            : `${PROVIDER_NAMES[provider]} refused to list its models. Type the model ID, or ask an administrator to check its key under AI providers.`}
        </p>
      )}
      <ModelField
        models={models}
        typed={typed}
        value={value}
        onPick={(model) =>
          onPick(
            model,
            models?.find((choice) => choice.id === model)?.context_window ??
              null,
          )
        }
      />
    </>
  )
}
