import hljs from 'highlight.js/lib/common'
import dockerfile from 'highlight.js/lib/languages/dockerfile'
import dart from 'highlight.js/lib/languages/dart'
import elixir from 'highlight.js/lib/languages/elixir'
import haskell from 'highlight.js/lib/languages/haskell'
import latex from 'highlight.js/lib/languages/latex'
import nginx from 'highlight.js/lib/languages/nginx'
import powershell from 'highlight.js/lib/languages/powershell'
import protobuf from 'highlight.js/lib/languages/protobuf'
import scala from 'highlight.js/lib/languages/scala'
import type { LanguageFn } from 'highlight.js'

const EXTRA: Record<string, LanguageFn> = {
  dockerfile,
  dart,
  elixir,
  haskell,
  latex,
  nginx,
  powershell,
  protobuf,
  scala,
}
for (const [name, language] of Object.entries(EXTRA))
  hljs.registerLanguage(name, language)

/** The code as escaped HTML with highlight spans; `null` when the language is unknown. */
export function highlighted(code: string, language: string) {
  if (!hljs.getLanguage(language)) return null
  return hljs.highlight(code, { language, ignoreIllegals: true }).value
}
