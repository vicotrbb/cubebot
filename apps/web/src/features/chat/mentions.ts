/** A name character, as the orchestrator's mention matcher counts them. */
const WORD = /[\p{L}\p{N}_-]/u

/** The `@` being typed at the caret: where it starts and what follows it so far. */
export type MentionQuery = { start: number; query: string }

/**
 * The mention the caret is in, if any. An `@` counts only at the start of the text or after
 * a non-name character, so `bob@example.com` never opens suggestions. Names may hold
 * spaces ("Test Agent"), so the query may too, but never a line break.
 */
export function mentionAt(text: string, caret: number): MentionQuery | null {
  const start = text.lastIndexOf('@', caret - 1)
  if (start < 0) return null
  const query = text.slice(start + 1, caret)
  if (query.length > 64 || query.includes('\n') || query.startsWith(' '))
    return null
  if (start > 0 && WORD.test(text[start - 1] ?? '')) return null
  return { start, query }
}

/** Members whose name, or one word of it, starts with the query; full-name matches first. */
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

/** The text with the mention replaced by `@name `, and the caret just after it. */
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
