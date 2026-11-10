import type { components } from '../../api/schema'

type Provider = components['schemas']['ProviderKind']
type Chosen = components['schemas']['ModelRef']

export type Draft = { provider: Provider; model: string; window: number | null }

export const DEFAULT = 'default'

export function draftFor(
  choice: Provider | typeof DEFAULT,
  saved: Chosen | null,
): Draft | null {
  if (choice === DEFAULT) return null
  if (saved?.provider !== choice)
    return { provider: choice, model: '', window: null }
  return {
    provider: choice,
    model: saved.model,
    window: saved.context_window ?? null,
  }
}

export function differs(draft: Draft | null, saved: Chosen | null) {
  return (
    (draft?.provider ?? null) !== (saved?.provider ?? null) ||
    (draft?.model ?? '') !== (saved?.model ?? '')
  )
}

export const ready = (draft: Draft | null) =>
  draft === null || draft.model !== ''
