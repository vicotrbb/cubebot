import { Marked, type Token, type TokenizerExtension } from 'marked'

/** Math between `$$`, `\\[` or `\\(`. A lone `$` stays a dollar sign: prices are far more common. A
 *  block has no `start`: with one, marked cuts a paragraph at any inline `$$` and rejoins it with
 *  line breaks. */
function math(
  name: string,
  level: 'block' | 'inline',
  pattern: RegExp,
  start?: string,
): TokenizerExtension {
  const extension: TokenizerExtension = {
    name,
    level,
    tokenizer(source) {
      const found = pattern.exec(source)
      if (!found) return undefined
      return { type: name, raw: found[0], text: (found[1] ?? '').trim() }
    },
  }
  if (!start) return extension
  return {
    ...extension,
    start: (source) => {
      const at = source.indexOf(start)
      return at < 0 ? undefined : at
    },
  }
}

const parser = new Marked({
  gfm: true,
  breaks: true,
  extensions: [
    math('mathBlock', 'block', /^\$\$([\s\S]+?)\$\$[ \t]*(?:\n|$)/),
    math('mathBlockAlt', 'block', /^\\\[([\s\S]+?)\\\][ \t]*(?:\n|$)/),
    math('mathInline', 'inline', /^\$\$([\s\S]+?)\$\$/, '$$'),
    math('mathInlineAlt', 'inline', /^\\\((.+?)\\\)/, '\\('),
  ],
})

export function tokensOf(text: string): Token[] {
  return parser.lexer(text)
}

/** A path on this origin. Browsers read `\` as `/` and drop tabs and line breaks, so each of
 *  those after the first slash can make a path that leaves it. */
const LOCAL_PATH = /^\/(?![\\/\t\n\r])/

/** A link's target when it may open: web, mail, or a path in this console. */
export function safeHref(href: string): string | null {
  if (href.startsWith('#') || LOCAL_PATH.test(href)) return href
  try {
    const { protocol } = new URL(href)
    return ['http:', 'https:', 'mailto:'].includes(protocol) ? href : null
  } catch {
    return null
  }
}

/** An image that may load by itself. An external one is a link instead: an agent reading
 *  untrusted pages could be tricked into writing an image URL that carries data out, and
 *  loading it would send that data with nobody clicking anything. */
export function inlineImage(href: string): string | null {
  if (LOCAL_PATH.test(href)) return href
  return /^data:image\/(png|jpe?g|gif|webp);base64,/i.test(href) ? href : null
}
