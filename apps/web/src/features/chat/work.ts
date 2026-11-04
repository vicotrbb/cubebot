import type { Turn } from '../../api/client'
import { grammarOf } from '../files/viewer'

type Progress = Turn['terminal'][number]

/** How a tool call reads to a person: a sentence, what it touched and what to show when opened. */
export type WorkView = {
  /** `Wrote`, `Opened`, `Ran`. */
  verb: string
  /** What it acted on: a file's name, a page, a command. */
  object: string
  /** Where the object sits, such as the file's folder. */
  where?: string
  /** A count beside the sentence, such as `+120 lines`. */
  stat?: string
  /** Which family's icon it gets. */
  family: 'file' | 'shell' | 'browser' | 'desktop' | 'knowledge' | 'other'
  /** The body shown when opened. */
  body:
    | {
        kind: 'file'
        path: string
        code: string
        language?: string | undefined
      }
    | { kind: 'diff'; path: string; before: string; after: string }
    | { kind: 'shell'; command: string }
    | { kind: 'output' }
    | { kind: 'fields'; fields: [string, string][] }
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

/** A browser call that names no page reads as what it did; its `session` is never news. */
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
    const code = output(tool)
    return fileView(
      'Read',
      path,
      code ? plural(lineCount(code), 'line') : undefined,
      { kind: 'file', path, code, language: grammarOf(path) },
    )
  },
  edit_file: (args) => {
    const path = text(args.path)
    const before = text(args.old_string)
    const after = text(args.new_string)
    return fileView(
      'Edited',
      path,
      `+${lineCount(after)} −${lineCount(before)}`,
      { kind: 'diff', path, before, after },
    )
  },
  glob: (args) => ({
    verb: 'Looked for',
    object: text(args.pattern),
    family: 'file',
    body: { kind: 'output' },
  }),
  grep: (args) => ({
    verb: 'Searched for',
    object: text(args.pattern),
    ...(text(args.path) ? { where: text(args.path) } : {}),
    family: 'file',
    body: { kind: 'output' },
  }),
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

/** Any other tool: its name in words, and its arguments as a short list instead of JSON. */
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

export type DiffLine = { sign: '+' | '-'; line: string }

/** The lines an edit took out and put in; what both keep at either end is not news. */
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

/** The turn in a phrase: files first, since that is what a person came for. */
export function workDigest(tools: Progress[]) {
  const views = tools.map(workView)
  const changed = new Set(
    views
      .filter((view) => view.body.kind === 'file' || view.body.kind === 'diff')
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
