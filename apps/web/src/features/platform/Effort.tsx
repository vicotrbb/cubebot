import { useId } from 'react'
import type { components } from '../../api/schema'
import { PROVIDER_NAMES } from './models'

type Provider = components['schemas']['ProviderKind']
export type Effort = components['schemas']['Effort']

const ALL: Effort[] = ['minimal', 'low', 'medium', 'high', 'xhigh', 'max']

export const EFFORTS: Record<Provider, Effort[]> = {
  openai: ALL,
  chatgpt: ALL,
  openrouter: ALL,
  anthropic: ['low', 'medium', 'high', 'xhigh', 'max'],
  gemini: ['minimal', 'low', 'medium', 'high', 'xhigh'],
  grok: ['low', 'medium', 'high', 'xhigh'],
  supergrok: [],
  minimax: [],
  minimax_cn: [],
  minimax_plan: [],
  minimax_plan_cn: [],
}

export const EFFORT_NAMES: Record<Effort, string> = {
  minimal: 'Minimal',
  low: 'Low',
  medium: 'Medium',
  high: 'High',
  xhigh: 'Extra high',
  max: 'Max',
}

const MEANING: Record<Effort, string> = {
  minimal: 'Answers fastest, with the least thinking.',
  low: 'Quick replies for simple, routine work.',
  medium: 'Balances speed and depth.',
  high: 'Thinks carefully before acting.',
  xhigh: 'Thinks longer on hard, multi-step work.',
  max: 'Thinks as long as it needs. Slowest and most expensive.',
}

export const offered = (provider: Provider, effort: Effort | null) =>
  effort === null || EFFORTS[provider].includes(effort)

export function EffortField({
  provider,
  value,
  inherited,
  onChange,
}: {
  provider: Provider
  value: Effort | null
  inherited: string
  onChange: (effort: Effort | null) => void
}) {
  const id = useId()
  const levels = EFFORTS[provider]
  const name = PROVIDER_NAMES[provider]
  if (levels.length === 0)
    return (
      <div className="effort">
        <span className="effort__label">Reasoning effort</span>
        <p className="small faint">
          {name} does not take a reasoning level, so its models decide how long
          to think.
        </p>
      </div>
    )
  return (
    <div className="effort">
      <span className="effort__label" id={id}>
        Reasoning effort
      </span>
      <div
        className="tabs effort__levels"
        role="radiogroup"
        aria-labelledby={id}
      >
        {[null, ...levels].map((level) => (
          <button
            key={level ?? 'default'}
            type="button"
            role="radio"
            aria-checked={value === level}
            onClick={() => onChange(level)}
          >
            {level ? EFFORT_NAMES[level] : 'Default'}
          </button>
        ))}
      </div>
      <p className="small faint">
        {value ? MEANING[value] : inherited} Higher levels think longer and cost
        more; a model that does not reason refuses any level, so keep Default
        for it.
      </p>
    </div>
  )
}
