import type { Turn } from '../../api/client'
import { grammarOf } from '../files/viewer'

type Progress = Turn['terminal'][number]

export type WorkView = {
  verb: string
  object: string
  where?: string
  stat?: string
  family: 'file' | 'shell' | 'browser' | 'desktop' | 'knowledge' | 'other'
  body:
    | {
        kind: 'file'
        path: string
        code: string
        language?: string | undefined
        start?: number
        note?: string
      }
    | { kind: 'diff'; path: string; before: string; after: string }
    | { kind: 'patch'; path: string; lines: PatchLine[]; after: string }
    | { kind: 'plan'; todos: Todo[] }
    | { kind: 'shell'; command: string }
    | { kind: 'output' }
    | { kind: 'fields'; fields: [string, string][] }
}

export type PatchLine = {
  sign: ' ' | '+' | '-'
  was?: number
  now?: number
  line: string
}

export type Todo = {
  content: string
  status: 'pending' | 'in_progress' | 'completed'
}

/** Arguments arrive as `name {json}`; a command that was cut or is not JSON has none. */
export function argumentsOf(tool: Pick<Progress, 'name' | 'command'>) {
  const text = tool.command.startsWith(`${tool.name} `)
    ? tool.command.slice(tool.name.length + 1)
    : tool.command
  try {
    const parsed: unknown = JSON.parse(text)
    return typeof parsed === 'object' && parsed !== null
      ? (parsed as Record<string, unknown>)
      : null
  } catch {
    return null
  }
}

const text = (value: unknown) => (typeof value === 'string' ? value : '')

function splitPath(path: string) {
  const slash = path.lastIndexOf('/')
  return slash < 0
    ? { name: path, folder: undefined }
    : { name: path.slice(slash + 1), folder: path.slice(0, slash) }
}

function lineCount(code: string) {
  if (!code) return 0
  return code.replace(/\n$/, '').split('\n').length
}

const plural = (count: number, word: string) =>
  `${count} ${word}${count === 1 ? '' : 's'}`

function fileView(
  verb: string,
  path: string,
  stat: string | undefined,
  body: WorkView['body'],
): WorkView {
  const { name, folder } = splitPath(path)
  return {
    verb,
    object: name,
    ...(folder ? { where: folder } : {}),
    ...(stat ? { stat } : {}),
    family: 'file',
    body,
  }
}

type Build = (args: Record<string, unknown>, tool: Progress) => WorkView

const output = (tool: Progress) =>
  tool.outcome && 'output' in tool.outcome ? tool.outcome.output : ''

function page(url: string) {
  try {
    return new URL(url).host + new URL(url).pathname.replace(/\/$/, '')
  } catch {
    return url
  }
}

const browser =
  (verb: string, object: (args: Record<string, unknown>) => string): Build =>
  (args) => ({
    verb,
    object: object(args),
    family: 'browser',
    body: { kind: 'output' },
  })

const knowledge =
  (verb: string, key: string): Build =>
  (args) => ({
    verb,
    object: text(args[key]),
    family: 'knowledge',
    body: { kind: 'output' },
  })

const filed =
  (verb: string): Build =>
  (args) => {
    const { name, folder } = splitPath(text(args.path) || text(args.name))
    return {
      verb,
      object: name,
      ...(folder ? { where: folder } : {}),
      family: 'file',
      body: { kind: 'output' },
    }
  }

const NUMBERED = /^ *(\d+)\t(.*)$/

export function numbered(printed: string) {
  const lines = printed.replace(/\n$/, '').split('\n')
  const first = NUMBERED.exec(lines[0] ?? '')
  if (!first) return { code: printed, start: 1, note: '' }
  const code: string[] = []
  const notes: string[] = []
  for (const line of lines) {
    const match = NUMBERED.exec(line)
    if (match) code.push(match[2] ?? '')
    else notes.push(line)
  }
  return {
    code: code.join('\n'),
    start: Number(first[1]),
    note: notes.join(' ').trim(),
  }
}

const HUNK = /^@@ -(\d+),\d+ \+(\d+),\d+ @@$/

export function patchOf(printed: string): PatchLine[] | null {
  const lines = printed.replace(/\n$/, '').split('\n')
  const at = lines.findIndex((line) => HUNK.test(line))
  if (at < 0) return null
  const head = HUNK.exec(lines[at] ?? '')
  let old = Number(head?.[1])
  let next = Number(head?.[2])
  return lines.slice(at + 1).flatMap((line): PatchLine[] => {
    const sign = line[0]
    if (sign === '-') return [{ sign, was: old++, line: line.slice(1) }]
    if (sign === '+') return [{ sign, now: next++, line: line.slice(1) }]
    if (sign === ' ')
      return [{ sign, was: old++, now: next++, line: line.slice(1) }]
    return []
  })
}

function todosOf(value: unknown): Todo[] {
  if (!Array.isArray(value)) return []
  return value.flatMap((item: unknown): Todo[] => {
    if (typeof item !== 'object' || item === null) return []
    const { content, status } = item as Record<string, unknown>
    const known = ['pending', 'in_progress', 'completed'].includes(text(status))
    return known && text(content)
      ? [{ content: text(content), status: status as Todo['status'] }]
      : []
  })
}

export function planSummary(todos: Todo[]) {
  const done = todos.filter((todo) => todo.status === 'completed').length
  return `${done} of ${todos.length} done`
}

function where(args: Record<string, unknown>) {
  const place = [text(args.path), text(args.glob)].filter(Boolean).join(', ')
  return place ? { where: place } : {}
}

const BUILDERS: Record<string, Build> = {
  write_file: (args) => {
    const path = text(args.path)
    const code = text(args.content)
    return fileView('Wrote', path, `${plural(lineCount(code), 'line')}`, {
      kind: 'file',
      path,
      code,
      language: grammarOf(path),
    })
  },
  read_file: (args, tool) => {
    const path = text(args.path)
    const failed = tool.outcome?.status === 'exited' && tool.outcome.code !== 0
    const { code, start, note } = numbered(failed ? '' : output(tool))
    const count = lineCount(code)
    const stat =
      start > 1 || note
        ? `lines ${start}–${start + Math.max(count, 1) - 1}`
        : plural(count, 'line')
    return fileView('Read', path, code ? stat : undefined, {
      kind: 'file',
      path,
      code,
      language: grammarOf(path),
      start,
      note,
    })
  },
  edit_file: (args, tool) => {
    const path = text(args.path)
    const before = text(args.old_string)
    const after = text(args.new_string)
    const lines = patchOf(output(tool))
    if (!lines)
      return fileView(
        'Edited',
        path,
        `+${lineCount(after)} −${lineCount(before)}`,
        { kind: 'diff', path, before, after },
      )
    const added = lines.filter((line) => line.sign === '+').length
    const removed = lines.filter((line) => line.sign === '-').length
    return fileView('Edited', path, `+${added} −${removed}`, {
      kind: 'patch',
      path,
      lines,
      after,
    })
  },
  glob: (args) => ({
    verb: 'Looked for',
    object: text(args.pattern),
    ...where({ path: args.path }),
    family: 'file',
    body: { kind: 'output' },
  }),
  grep: (args) => ({
    verb: 'Searched for',
    object: text(args.pattern),
    ...where(args),
    family: 'file',
    body: { kind: 'output' },
  }),
  todo_write: (args) => {
    const todos = todosOf(args.todos)
    return {
      verb: 'Updated the plan',
      object: planSummary(todos),
      family: 'other',
      body: { kind: 'plan', todos },
    }
  },
  browser_navigate: browser('Opened', (args) => page(text(args.url))),
  browser_read: browser('Read', () => 'the page'),
  browser_click: browser('Clicked', (args) =>
    args.node === undefined ? 'the page' : `element ${String(args.node)}`,
  ),
  browser_type: browser('Typed', (args) => `“${text(args.text)}”`),
  browser_screenshot: browser('Looked at', () => 'the page'),
  browser_close: browser('Closed', () => 'the page'),
  kb_search: knowledge('Searched the knowledge base for', 'query'),
  kb_read: knowledge('Read a knowledge base page', 'title'),
  kb_create: knowledge('Added to the knowledge base', 'title'),
  kb_write: knowledge('Rewrote a knowledge base page', 'title'),
  kb_edit: knowledge('Edited a knowledge base page', 'title'),
  search_memory: knowledge('Searched its memory for', 'query'),
  remember: knowledge('Remembered', 'text'),
  load_skill: knowledge('Loaded the skill', 'name'),
  files_put: filed('Saved to Files'),
  files_get: filed('Fetched from Files'),
  files_share: filed('Shared'),
  send_attachment: filed('Sent'),
  task_message: (args) => ({
    verb: 'Told the room',
    object: text(args.message),
    family: 'other',
    body: { kind: 'output' },
  }),
  task_item: (args) => ({
    verb: `Marked item #${String(args.item)}`,
    object: text(args.status),
    family: 'other',
    body: { kind: 'output' },
  }),
}

function generic(
  tool: Progress,
  args: Record<string, unknown> | null,
): WorkView {
  const words = tool.name.replaceAll('_', ' ')
  const verb = words.charAt(0).toUpperCase() + words.slice(1)
  const fields = Object.entries(args ?? {})
    .filter(([key, value]) => key !== 'session' && value !== null)
    .map(([key, value]): [string, string] => [
      key.replaceAll('_', ' '),
      typeof value === 'string' ? value : JSON.stringify(value),
    ])
  return {
    verb,
    object: fields[0]?.[1] ?? '',
    family: 'other',
    body: { kind: 'fields', fields },
  }
}

export function workView(tool: Progress): WorkView {
  if (tool.name === 'bash' || tool.name === 'terminal')
    return {
      verb: 'Ran',
      object: tool.command.split('\n')[0] ?? '',
      family: 'shell',
      body: { kind: 'shell', command: tool.command },
    }
  if (
    ['computer', 'screenshot', 'vault_fill', 'request_login'].includes(
      tool.name,
    )
  )
    return { ...generic(tool, argumentsOf(tool)), family: 'desktop' }
  const args = argumentsOf(tool)
  const build = BUILDERS[tool.name]
  return build && args ? build(args, tool) : generic(tool, args)
}

type DiffLine = { sign: '+' | '-'; line: string }

export function diffLines(before: string, after: string): DiffLine[] {
  const a = before.split('\n')
  const b = after.split('\n')
  let start = 0
  while (start < a.length && start < b.length && a[start] === b[start]) start++
  let end = 0
  while (
    end < a.length - start &&
    end < b.length - start &&
    a[a.length - 1 - end] === b[b.length - 1 - end]
  )
    end++
  return [
    ...a
      .slice(start, a.length - end)
      .map((line): DiffLine => ({ sign: '-', line })),
    ...b
      .slice(start, b.length - end)
      .map((line): DiffLine => ({ sign: '+', line })),
  ]
}

export function workDigest(tools: Progress[]) {
  const views = tools.map(workView)
  const changed = new Set(
    views
      .filter((view) => ['file', 'diff', 'patch'].includes(view.body.kind))
      .filter((view) => view.verb !== 'Read')
      .map((view) =>
        view.where ? `${view.where}/${view.object}` : view.object,
      ),
  )
  const commands = views.filter((view) => view.family === 'shell').length
  return [
    changed.size ? `${plural(changed.size, 'file')} changed` : '',
    commands ? plural(commands, 'command') : '',
  ]
    .filter(Boolean)
    .join(' · ')
}

export function latestPlan(tools: Progress[]) {
  const last = tools.filter((tool) => tool.name === 'todo_write').at(-1)
  const args = last && argumentsOf(last)
  return args ? todosOf(args.todos) : null
}
