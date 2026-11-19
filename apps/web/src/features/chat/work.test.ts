// @ts-expect-error bun provides this at test time
import { describe, expect, test } from 'bun:test'
import {
  argumentsOf,
  diffLines,
  latestPlan,
  numbered,
  patchOf,
  workDigest,
  workView,
} from './work'

const tool = (name: string, input: unknown, output?: string) =>
  ({
    id: name,
    turn_id: 't',
    name,
    command:
      name === 'bash'
        ? String(input)
        : `${name} ${typeof input === 'string' ? input : JSON.stringify(input)}`,
    status: 'exited',
    outcome:
      output === undefined ? null : { status: 'exited', code: 0, output },
    reobserved: false,
  }) as Parameters<typeof workView>[0]

describe('workView', () => {
  test('a written file is its name, folder, size and highlighted content', () => {
    const view = workView(
      tool('write_file', { path: 'finder/subtitles.ts', content: 'a\nb\n' }),
    )
    expect(view.verb).toBe('Wrote')
    expect(view.object).toBe('subtitles.ts')
    expect(view.where).toBe('finder')
    expect(view.stat).toBe('2 lines')
    expect(view.body).toEqual({
      kind: 'file',
      path: 'finder/subtitles.ts',
      code: 'a\nb\n',
      language: 'typescript',
    })
  })
  test('an edit reads as a diff with its counts', () => {
    const view = workView(
      tool('edit_file', { path: 'a.py', old_string: 'x', new_string: 'y\nz' }),
    )
    expect(view.stat).toBe('+2 −1')
    expect(view.body.kind).toBe('diff')
  })
  test('browser calls read as what they did and never mention their session', () => {
    const opened = workView(
      tool('browser_navigate', {
        url: 'https://a.example/docs/',
        session: 'main',
      }),
    )
    expect(`${opened.verb} ${opened.object}`).toBe('Opened a.example/docs')
    const read = workView(tool('browser_read', { session: 'main' }))
    expect(`${read.verb} ${read.object}`).toBe('Read the page')
  })
  test('a command keeps its whole text and shows its first line', () => {
    const view = workView(tool('bash', 'cd x\nls'))
    expect(view.object).toBe('cd x')
    expect(view.body).toEqual({ kind: 'shell', command: 'cd x\nls' })
  })
  test('an unknown tool lists its arguments, and truncated JSON still reads', () => {
    const view = workView(
      tool('vault_list', { path: 'r.pdf', session: 'main' }),
    )
    expect(view.verb).toBe('Vault list')
    expect(view.body).toEqual({ kind: 'fields', fields: [['path', 'r.pdf']] })
    expect(argumentsOf(tool('write_file', '{"path":"a'))).toBeNull()
    expect(workView(tool('write_file', '{"path":"a')).verb).toBe('Write file')
  })
})

test('diffLines keeps only what changed', () => {
  expect(diffLines('a\nb\nc', 'a\nB\nc')).toEqual([
    { sign: '-', line: 'b' },
    { sign: '+', line: 'B' },
  ])
  expect(diffLines('same', 'same')).toEqual([])
  expect(diffLines('', 'new')).toEqual([
    { sign: '-', line: '' },
    { sign: '+', line: 'new' },
  ])
})

test('workDigest counts changed files once and commands', () => {
  expect(
    workDigest([
      tool('write_file', { path: 'a.ts', content: '' }),
      tool('edit_file', { path: 'a.ts', old_string: 'a', new_string: 'b' }),
      tool('read_file', { path: 'b.ts' }),
      tool('bash', 'ls'),
    ]),
  ).toBe('1 file changed · 1 command')
})

describe('the coding harness views', () => {
  test('a numbered read starts at its first line and keeps the paging note', () => {
    const printed =
      '    40\tfn main() {\n    41\t\tprintln!("hi");\n[lines 40-41 of 90; continue with offset 42]\n'
    expect(numbered(printed)).toEqual({
      code: 'fn main() {\n\tprintln!("hi");',
      start: 40,
      note: '[lines 40-41 of 90; continue with offset 42]',
    })
    expect(numbered('plain\ntext')).toEqual({
      code: 'plain\ntext',
      start: 1,
      note: '',
    })
    const view = workView(tool('read_file', { path: 'src/main.rs' }, printed))
    expect(view.stat).toBe('lines 40–41')
    expect(view.body).toMatchObject({ kind: 'file', start: 40 })
  })

  test('an edit shows the diff the tool printed, with both line numbers', () => {
    const printed =
      'Replaced 1 occurrence in a.ts\n@@ -2,3 +2,4 @@\n keep\n-old\n+new\n+more\n tail'
    expect(patchOf(printed)).toEqual([
      { sign: ' ', was: 2, now: 2, line: 'keep' },
      { sign: '-', was: 3, line: 'old' },
      { sign: '+', now: 3, line: 'new' },
      { sign: '+', now: 4, line: 'more' },
      { sign: ' ', was: 4, now: 5, line: 'tail' },
    ])
    const view = workView(
      tool(
        'edit_file',
        { path: 'a.ts', old_string: 'old', new_string: 'new\nmore' },
        printed,
      ),
    )
    expect(view.stat).toBe('+2 −1')
    expect(view.body.kind).toBe('patch')
    expect(patchOf('old_string not found in a.ts')).toBe(null)
  })

  test('the latest plan wins and unknown steps are dropped', () => {
    const plan = (todos: unknown) => tool('todo_write', { todos })
    const latest = plan([
      { content: 'Read', status: 'completed' },
      { content: 'Fix', status: 'in_progress' },
      { content: 'Odd', status: 'maybe' },
    ])
    expect(
      latestPlan([plan([{ content: 'Read', status: 'in_progress' }]), latest]),
    ).toEqual([
      { content: 'Read', status: 'completed' },
      { content: 'Fix', status: 'in_progress' },
    ])
    expect(workView(latest).object).toBe('1 of 2 done')
    expect(latestPlan([tool('bash', 'ls')])).toBe(null)
  })
})
