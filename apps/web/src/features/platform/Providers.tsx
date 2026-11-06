import { useQuery, useQueryClient } from '@tanstack/react-query'
import { Check, ChevronDown, KeyRound, RefreshCw, Search } from 'lucide-react'
import { useMemo, useState } from 'react'
import { api, failure, ok } from '../../api/client'
import type { components } from '../../api/schema'
import { Alert, Field, Skeleton, Status } from '../../components/primitives'
import { COMMON_WINDOWS, suggestedWindow, tokens } from './models'
import { PLANS, PlanBlock, type PlanKind } from './Plan'
import './platform.css'

type Provider = components['schemas']['ProviderKind']
type Current = components['schemas']['ProviderSummary']
type Choice = components['schemas']['ModelChoice']
type Saved = components['schemas']['SavedProvider']
type Subscription = components['schemas']['SubscriptionSummary']

const PROVIDERS: {
  id: Provider
  name: string
  note: string
  plan?: PlanKind
}[] = [
  {
    id: 'openai',
    name: 'OpenAI',
    note: 'A ChatGPT plan, or the Responses API with a key.',
    plan: 'chatgpt',
  },
  {
    id: 'anthropic',
    name: 'Anthropic',
    note: 'The Messages API. Each model reports its own window.',
  },
  {
    id: 'openrouter',
    name: 'OpenRouter',
    note: 'Hundreds of models from many labs, on one key.',
  },
  {
    id: 'gemini',
    name: 'Google Gemini',
    note: 'A key from Google AI Studio. Each model reports its own window.',
  },
  {
    id: 'grok',
    name: 'xAI Grok',
    note: 'A SuperGrok plan, or a key from the xAI console.',
    plan: 'supergrok',
  },
  {
    id: 'minimax',
    name: 'MiniMax',
    note: 'An M Plan key, or a pay-as-you-go key from platform.minimax.io.',
    plan: 'minimax_plan',
  },
  {
    id: 'minimax_cn',
    name: 'MiniMax mainland China',
    note: 'Keys from platform.minimaxi.com, served from api.minimax.cn.',
    plan: 'minimax_plan_cn',
  },
]
const NAMES = {
  ...Object.fromEntries(PROVIDERS.map(({ id, name }) => [id, name])),
  chatgpt: 'your ChatGPT plan',
  supergrok: 'your Grok plan',
  minimax_plan: 'your MiniMax plan',
  minimax_plan_cn: 'your MiniMax plan',
} as Record<Provider, string>

type Standing = 'active' | 'plan' | 'saved' | 'none'
const STANDING: Record<Standing, string> = {
  active: 'In use',
  plan: 'Plan signed in',
  saved: 'Key saved',
  none: 'Not configured',
}

export function Providers() {
  const client = useQueryClient()
  const current = useQuery({
    queryKey: ['provider'],
    queryFn: () => ok(api.GET('/api/provider')),
  })
  const saved = useQuery({
    queryKey: ['provider-saved'],
    queryFn: () => ok(api.GET('/api/provider/saved')),
  })
  const plans = useQuery({
    queryKey: ['provider-subscriptions'],
    queryFn: () => ok(api.GET('/api/provider/subscriptions')),
    // Renewals and usage-limit waits change without anyone touching this page.
    refetchInterval: 30_000,
  })
  const [open, setOpen] = useState<Provider | null>(null)
  // Here, not in a form, which starts over on the revision a save makes.
  const [notice, setNotice] = useState('')
  const refresh = () =>
    Promise.all([
      client.invalidateQueries({ queryKey: ['provider'] }),
      client.invalidateQueries({ queryKey: ['provider-saved'] }),
      client.invalidateQueries({ queryKey: ['provider-subscriptions'] }),
    ])
  if (current.isPending || saved.isPending || plans.isPending)
    return <Skeleton lines={4} width="62ch" />
  if (current.isError || saved.isError || plans.isError)
    return <Alert>The providers could not be read. Retry shortly.</Alert>
  return (
    <div className="provider-page">
      <p className="platform__lead">
        Every eligible agent thinks with the provider in use. Keys and plan
        sign-ins are encrypted on the server and never shown again.
      </p>
      <Status>{notice}</Status>
      <div className="provider-list">
        {PROVIDERS.map((provider) => (
          <ProviderCard
            key={provider.id}
            {...provider}
            current={current.data ?? null}
            saved={saved.data.find((item) => item.provider === provider.id)}
            subscription={plans.data.find(
              (item) => item.provider === provider.plan,
            )}
            open={open === provider.id}
            onToggle={() => {
              setNotice('')
              setOpen(open === provider.id ? null : provider.id)
            }}
            onDone={async (message) => {
              setNotice(message)
              setOpen(null)
              await refresh()
            }}
            onPlanChanged={async (message) => {
              setNotice(message)
              await refresh()
            }}
          />
        ))}
      </div>
    </div>
  )
}

function ProviderCard({
  id,
  name,
  note,
  plan,
  current,
  saved,
  subscription,
  open,
  onToggle,
  onDone,
  onPlanChanged,
}: {
  id: Provider
  name: string
  note: string
  plan?: PlanKind | undefined
  current: Current | null
  saved: Saved | undefined
  subscription: Subscription | undefined
  open: boolean
  onToggle: () => void
  onDone: (message: string) => Promise<unknown>
  onPlanChanged: (message: string) => Promise<unknown>
}) {
  const planInUse = Boolean(plan) && current?.provider === plan
  const active = current?.provider === id || planInUse ? current : null
  const standing = cardStanding(Boolean(active), subscription, saved)
  const choice = active ?? saved
  const panel = `provider-${id}`
  return (
    <section
      className="provider-card"
      data-open={open}
      data-standing={standing}
    >
      <h2>
        <button
          type="button"
          className="provider-card__head"
          aria-expanded={open}
          aria-controls={panel}
          onClick={onToggle}
        >
          <span className="provider-card__name">
            <strong>{name}</strong>
            <span className="small faint">{note}</span>
          </span>
          {choice && (
            <span className="provider-card__choice">
              <code>{choice.model}</code>
              {choice.context_window && (
                <span className="small faint">
                  {tokens(choice.context_window)} window
                </span>
              )}
            </span>
          )}
          <span className="provider-card__standing">
            <span className="provider-mark" aria-hidden="true" />
            {STANDING[standing]}
          </span>
          <ChevronDown className="provider-card__chevron" aria-hidden="true" />
        </button>
      </h2>
      {open && (
        <div id={panel} className="provider-card__body">
          {plan && (
            <>
              <PlanBlock
                plan={plan}
                subscription={subscription}
                inUse={planInUse}
                onChanged={onPlanChanged}
              >
                <ProviderForm
                  key={`plan-${current?.revision ?? 0}`}
                  provider={plan}
                  keyless
                  current={current}
                  saved={undefined}
                  onSaved={() =>
                    onDone(
                      planInUse
                        ? `Your ${PLANS[plan].name} plan saved. Agents use it from their next turn.`
                        : `Your agents now use your ${PLANS[plan].name} plan, from their next turn.`,
                    )
                  }
                  onForgotten={() => onDone('')}
                />
              </PlanBlock>
              <h3 className="provider-card__or">{orKey(plan)}</h3>
            </>
          )}
          <ProviderForm
            key={current?.revision ?? 0}
            provider={id}
            current={current}
            saved={saved}
            onSaved={() =>
              onDone(
                active
                  ? `${name} saved. Agents use it from their next turn.`
                  : `Agents now use ${name}, from their next turn.`,
              )
            }
            onForgotten={() => onDone(`The ${name} key is gone.`)}
          />
        </div>
      )}
    </section>
  )
}

function cardStanding(
  active: boolean,
  subscription: Subscription | undefined,
  saved: Saved | undefined,
): Standing {
  if (active) return 'active'
  if (subscription) return 'plan'
  return saved ? 'saved' : 'none'
}

const orKey = (plan: PlanKind) =>
  PLANS[plan].keyed ? 'Or use a pay-as-you-go key' : 'Or use an API key'

export function useModels(provider: Provider, key: string, saved: boolean) {
  const [asked, setAsked] = useState<string | null>(null)
  const listing = saved && !key ? '' : asked
  const query = useQuery({
    queryKey: ['provider-models', provider, listing],
    enabled: listing !== null,
    retry: false,
    // A pasted key lives in this cache only while the form shows its models.
    gcTime: 0,
    queryFn: async () => {
      const { data, response } = await api
        .POST('/api/provider/models', {
          body: { provider, api_key: listing || null },
        })
        .catch(() => ({ data: undefined, response: null }))
      if (!data)
        throw new Error(listProblem(response?.status, provider), {
          cause: unreachable(response?.status),
        })
      return data
    },
  })
  return {
    models: query.data ?? null,
    problem: query.error?.message ?? '',
    // Unreachable, not refused: a person may type the model instead.
    unreachable: query.error?.cause === true,
    loading: query.isFetching,
    load: () => {
      if (listing === (key || (saved ? '' : null))) void query.refetch()
      else setAsked(key)
    },
    reset: () => setAsked(null),
  }
}

function draftFor(choice: Current | Saved | null | undefined) {
  return {
    model: choice?.model ?? '',
    window: choice?.context_window ? String(choice.context_window) : '',
  }
}

/** One request id per attempt, so a retried save is not a second one. */
function useSaveConnection(
  current: Current | null,
  onSaved: () => Promise<unknown>,
) {
  const [error, setError] = useState('')
  const [busy, setBusy] = useState(false)
  const [requestId, setRequestId] = useState(() => crypto.randomUUID())
  async function save(body: {
    provider: Provider
    model: string
    window: string
    key: string
  }) {
    setBusy(true)
    setError('')
    try {
      await ok(
        api.POST('/api/provider', {
          body: {
            request_id: requestId,
            expected_revision: current?.revision ?? 0,
            provider: body.provider,
            model: body.model,
            context_window: body.window === '' ? null : Number(body.window),
            api_key: body.key || null,
          },
        }),
      )
      setRequestId(crypto.randomUUID())
      await onSaved()
    } catch (error) {
      setError(failure(error))
    } finally {
      setBusy(false)
    }
  }
  return { save, error, busy }
}

const unreachable = (status: number | undefined) =>
  status === 502 || status === undefined

function listProblem(status: number | undefined, provider: Provider) {
  if (status === 422)
    return `${NAMES[provider]} rejected this key. Check it and try again.`
  if (unreachable(status))
    return `${NAMES[provider]} could not be reached. Type the model ID instead, or retry.`
  if (status === 400) return 'That key has characters a key cannot hold.'
  return 'The models could not be listed. Retry shortly.'
}

function ProviderForm({
  provider,
  keyless = false,
  current,
  saved,
  onSaved,
  onForgotten,
}: {
  provider: Provider
  /** A signed-in plan: its own token lists the models, and there is no key to paste. */
  keyless?: boolean
  current: Current | null
  saved: Saved | undefined
  onSaved: () => Promise<unknown>
  onForgotten: () => Promise<unknown>
}) {
  const active = current?.provider === provider
  const hasKey = keyless || active || Boolean(saved)
  const [key, setKey] = useState('')
  const [draft, setDraft] = useState(() => draftFor(active ? current : saved))
  const list = useModels(provider, key, hasKey)
  const connection = useSaveConnection(current, onSaved)
  const forget = useForget(provider, onForgotten)
  const choose = (id: string) => {
    const reported = list.models?.find((choice) => choice.id === id)
    const suggestion = suggestedWindow(id, reported?.context_window)
    setDraft({ model: id, window: suggestion ? String(suggestion.tokens) : '' })
  }
  const ready = [
    draft.model,
    key || hasKey,
    provider === 'anthropic' || draft.window,
  ].every(Boolean)
  const name = NAMES[provider]
  return (
    <form
      className="form provider-form"
      onSubmit={(event) => {
        event.preventDefault()
        void connection.save({ provider, key, ...draft })
      }}
    >
      {!keyless && (
        <KeyField
          provider={provider}
          value={key}
          saved={hasKey}
          loading={list.loading}
          onChange={(value) => {
            setKey(value)
            list.reset()
          }}
          onLoad={() => void list.load()}
        />
      )}
      {list.problem && (
        <p role="alert" className="small provider-problem">
          {list.problem}
        </p>
      )}
      <ModelField
        models={list.models}
        planOrder={keyless}
        typed={list.unreachable}
        value={draft.model}
        onPick={choose}
      />
      <WindowField
        provider={provider}
        model={draft.model}
        reported={
          list.models?.find((choice) => choice.id === draft.model)
            ?.context_window
        }
        value={draft.window}
        onChange={(window) => setDraft({ ...draft, window })}
      />
      <Alert>{connection.error || forget.error}</Alert>
      <div className="actions">
        <button
          className="btn btn--primary"
          disabled={connection.busy || !ready}
        >
          {connection.busy ? 'Saving…' : saveLabel(active, current, name)}
        </button>
        {saved && (
          <button
            type="button"
            className="btn btn--ghost"
            disabled={forget.busy}
            onClick={() => void forget.run()}
          >
            Forget this key
          </button>
        )}
      </div>
    </form>
  )
}

function saveLabel(active: boolean, current: Current | null, name: string) {
  if (active) return 'Save changes'
  return current ? `Switch agents to ${name}` : `Use ${name}`
}

function useForget(provider: Provider, onDone: () => Promise<unknown>) {
  const [error, setError] = useState('')
  const [busy, setBusy] = useState(false)
  const run = async () => {
    setBusy(true)
    setError('')
    try {
      await ok(
        api.DELETE('/api/provider/saved/{provider}', {
          params: { path: { provider } },
        }),
      )
      await onDone()
    } catch (error) {
      setError(failure(error))
    } finally {
      setBusy(false)
    }
  }
  return { run, error, busy }
}

function KeyField({
  provider,
  value,
  saved,
  loading,
  onChange,
  onLoad,
}: {
  provider: Provider
  value: string
  saved: boolean
  loading: boolean
  onChange: (value: string) => void
  onLoad: () => void
}) {
  return (
    <div className="provider-key">
      <Field
        label="API key"
        hint={saved ? 'Empty keeps the saved key' : 'Never shown again'}
      >
        <input
          type="password"
          autoComplete="off"
          spellCheck={false}
          maxLength={4096}
          value={value}
          placeholder={
            saved
              ? 'Saved key in use; paste a new one to replace it'
              : `Your ${NAMES[provider]} API key`
          }
          onChange={(event) => onChange(event.target.value.trim())}
          onBlur={() => value && onLoad()}
          onKeyDown={(event) => {
            if (event.key !== 'Enter') return
            event.preventDefault()
            onLoad()
          }}
        />
      </Field>
      <button
        type="button"
        className="btn"
        disabled={loading || (!value && !saved)}
        onClick={onLoad}
      >
        {loading ? <RefreshCw className="spin" /> : <KeyRound />}
        {loading ? 'Checking…' : 'List models'}
      </button>
    </div>
  )
}

export function ModelField({
  models,
  planOrder = false,
  typed,
  value,
  onPick,
}: {
  models: Choice[] | null
  /** A plan lists its models in its own order, not by date. */
  planOrder?: boolean
  typed: boolean
  value: string
  onPick: (id: string) => void
}) {
  const [search, setSearch] = useState('')
  const matching = useMemo(() => {
    const words = search.toLowerCase().split(/\s+/).filter(Boolean)
    return (models ?? []).filter((choice) =>
      words.every((word) => choice.id.toLowerCase().includes(word)),
    )
  }, [models, search])
  if (typed)
    return (
      <Field label="Model">
        <input
          required
          maxLength={128}
          value={value}
          placeholder="A model ID the key can use"
          onChange={(event) => onPick(event.target.value.trim())}
        />
      </Field>
    )
  if (!models)
    return (
      <Field label="Model">
        <input
          disabled
          value={value}
          placeholder="Enter the key to list its models"
        />
      </Field>
    )
  const chosen = models.find((choice) => choice.id === value)
  return (
    <fieldset className="provider-models">
      <legend>
        Model
        <span className="hint">
          {models.length} available,{' '}
          {planOrder ? 'in the plan’s order' : 'newest first'}
        </span>
      </legend>
      <div className="provider-models__search">
        <Search aria-hidden="true" />
        <input
          className="input"
          type="search"
          aria-label="Search models"
          placeholder="Search models"
          value={search}
          onChange={(event) => setSearch(event.target.value)}
          onKeyDown={(event) => {
            if (event.key !== 'Enter') return
            event.preventDefault()
            if (matching[0]) onPick(matching[0].id)
          }}
        />
      </div>
      <div
        className="provider-models__list"
        role="radiogroup"
        aria-label="Model"
      >
        {matching.map((choice) => (
          <label key={choice.id} className="provider-model">
            <input
              type="radio"
              name="model"
              checked={choice.id === value}
              onChange={() => onPick(choice.id)}
            />
            <Check className="provider-model__mark" aria-hidden="true" />
            <code>{choice.id}</code>
            {!choice.tools && <span className="chip">No tools</span>}
            {choice.context_window && (
              <span className="small faint tabular">
                {tokens(choice.context_window)}
              </span>
            )}
          </label>
        ))}
        {matching.length === 0 && (
          <p className="small faint provider-models__none">
            No model matches that.
          </p>
        )}
      </div>
      {value && !chosen && (
        <p className="small faint">
          In use: <code>{value}</code>, which this key no longer lists.
        </p>
      )}
      {chosen && !chosen.tools && (
        <p className="small provider-problem" role="status">
          This model cannot call tools, so an agent on it can only talk.
        </p>
      )}
    </fieldset>
  )
}

function WindowField({
  provider,
  model,
  reported,
  value,
  onChange,
}: {
  provider: Provider
  model: string
  reported: number | null | undefined
  value: string
  onChange: (value: string) => void
}) {
  const suggestion = model ? suggestedWindow(model, reported) : null
  const sizes = [
    ...new Set([...(suggestion ? [suggestion.tokens] : []), ...COMMON_WINDOWS]),
  ]
  return (
    <div className="provider-group">
      <Field label="Context window" hint={windowHint(provider, suggestion)}>
        <input
          type="number"
          inputMode="numeric"
          min={1024}
          max={100000000}
          required={provider !== 'anthropic'}
          value={value}
          placeholder={
            provider === 'anthropic' ? 'Reported by Anthropic' : 'Tokens'
          }
          onChange={(event) => onChange(event.target.value)}
        />
      </Field>
      <span
        className="provider-sizes"
        role="group"
        aria-label="Suggested sizes"
      >
        {sizes.map((size) => (
          <button
            key={size}
            type="button"
            className="chip provider-size"
            aria-pressed={value === String(size)}
            onClick={() => onChange(String(size))}
          >
            {tokens(size)}
            {size === suggestion?.tokens && (
              <span className="provider-size__tag">
                {suggestion.source === 'reported' ? 'reported' : 'suggested'}
              </span>
            )}
          </button>
        ))}
      </span>
      <span className="small faint">
        Conversations are summarized at 80% of the window.
      </span>
    </div>
  )
}

function windowHint(
  provider: Provider,
  suggestion: ReturnType<typeof suggestedWindow>,
) {
  if (suggestion?.source === 'reported') return 'Reported by the provider'
  if (suggestion) return "Typical for this model's family"
  return provider === 'anthropic' ? 'Empty uses the model’s report' : undefined
}
