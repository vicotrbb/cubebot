// @ts-expect-error bun provides this at test time
import { describe, expect, test } from 'bun:test'
import { argumentsOf, diffLines, workDigest, workView } from './work'

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
