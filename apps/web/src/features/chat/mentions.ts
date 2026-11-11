/** As the orchestrator's mention matcher counts them. */
const WORD = /[\p{L}\p{N}_-]/u

export type MentionQuery = { start: number; query: string }

/** An `@` counts only at the start or after a non-name character, so `bob@example.com` never opens suggestions; names may hold spaces, so the query may too, but never a line break. */
export function mentionAt(text: string, caret: number): MentionQuery | null {
  const start = text.lastIndexOf('@', caret - 1)
  if (start < 0) return null
  const query = text.slice(start + 1, caret)
  if (query.length > 64 || query.includes('\n') || query.startsWith(' '))
    return null
  if (start > 0 && WORD.test(text[start - 1] ?? '')) return null
  return { start, query }
}

export function suggest<T extends { name: string }>(
  members: T[],
  query: string,
): T[] {
  const wanted = query.toLowerCase()
  const whole = (member: T) => member.name.toLowerCase().startsWith(wanted)
  const word = (member: T) =>
    member.name
      .toLowerCase()
      .split(/\s+/)
      .some((part) => part.startsWith(wanted))
  return [
    ...members.filter(whole),
    ...members.filter((member) => !whole(member) && word(member)),
  ]
}

export function complete(
  text: string,
  caret: number,
  mention: MentionQuery,
  name: string,
) {
  const inserted = `@${name} `
  const rest = text.slice(caret).replace(/^ /, '')
  return {
    text: text.slice(0, mention.start) + inserted + rest,
    caret: mention.start + inserted.length,
  }
}

export type Span = { text: string; mention: boolean }

export function mentionSpans(text: string, names: string[]): Span[] {
  const longest = [...names].sort((a, b) => b.length - a.length)
  const spans: Span[] = []
  let from = 0
  for (let at = text.indexOf('@'); at >= 0; at = text.indexOf('@', at + 1)) {
    if (at < from || (at > 0 && WORD.test(text[at - 1] ?? ''))) continue
    const name = longest.find((candidate) => named(text, at + 1, candidate))
    if (!name) continue
    if (at > from) spans.push({ text: text.slice(from, at), mention: false })
    spans.push({ text: text.slice(at, at + 1 + name.length), mention: true })
    from = at + 1 + name.length
  }
  if (from < text.length) spans.push({ text: text.slice(from), mention: false })
  return spans
}

function named(text: string, at: number, name: string) {
  const end = at + name.length
  return (
    name.length > 0 &&
    text.slice(at, end).toLowerCase() === name.toLowerCase() &&
    !WORD.test(text[end] ?? '')
  )
}
