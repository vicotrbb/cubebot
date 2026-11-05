/** OpenAI reports no context windows, so its families get the input size their documentation gives. */
const FAMILIES: [RegExp, number][] = [
  [/^gpt-4\.1/, 1_047_576],
  // Input limit: the 400K total holds 128K of output.
  [/^gpt-5/, 272_000],
  [/^o[134](-|$)/, 200_000],
  [/^(gpt-4o|chatgpt-4o|gpt-4-turbo)/, 128_000],
]
export const COMMON_WINDOWS = [128_000, 200_000, 272_000, 1_000_000]

export function suggestedWindow(
  model: string,
  reported: number | null | undefined,
): { tokens: number; source: 'reported' | 'family' } | null {
  if (reported) return { tokens: reported, source: 'reported' }
  const family = FAMILIES.find(([pattern]) => pattern.test(model))
  return family ? { tokens: family[1], source: 'family' } : null
}

export function tokens(count: number) {
  if (count >= 1_000_000) return `${Number((count / 1_000_000).toFixed(2))}M`
  return `${Math.round(count / 1000)}K`
}
