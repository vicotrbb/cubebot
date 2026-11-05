// @ts-expect-error bun provides this at test time
import { describe, expect, test } from 'bun:test'
import { inlineImage, safeHref, tokensOf } from './tokens'

const types = (text: string) => tokensOf(text).map((token) => token.type)

describe('tokensOf', () => {
  test('reads the formats models write', () => {
    const kinds = types('# T\n\n- a\n\n1. b\n\n> c\n\n---').filter(
      (kind) => kind !== 'space',
    )
    expect(kinds).toEqual(['heading', 'list', 'list', 'blockquote', 'hr'])
    expect(types('| a |\n| - |\n| 1 |')).toEqual(['table'])
    expect(types('```rust\nfn x() {}\n```')).toEqual(['code'])
  })
  test('bold inside a numbered list stays bold', () => {
    const [list] = tokensOf('1. **Apr 12** — *Book*') as never as [
      { items: { tokens: { tokens: { type: string }[] }[] }[] },
    ]
    const inline = list.items[0]?.tokens[0]?.tokens.map((token) => token.type)
    expect(inline).toEqual(['strong', 'text', 'em'])
  })
  test('math needs its delimiters; a price is not math', () => {
    expect(types('$$x^2$$')).toEqual(['mathBlock'])
    const [mixed] = tokensOf('Energy is $$E = mc^2$$ here') as never as [
      { tokens: { type: string }[] },
    ]
    expect(mixed.tokens.map((token) => token.type)).toEqual([
      'text',
      'mathInline',
      'text',
    ])
    const [line] = tokensOf('It costs $5 and $10') as never as [
      { tokens: { type: string }[] },
    ]
    expect(line.tokens.map((token) => token.type)).toEqual(['text'])
  })
  test('a half-streamed message still parses', () => {
    expect(types('```js\nconst a').at(-1)).toBe('code')
    expect(() => tokensOf('**half')).not.toThrow()
  })
})

describe('safeHref', () => {
  test('opens the web, mail and console paths', () => {
    for (const href of ['https://x.io/a', 'mailto:a@b.c', '/files', '#top'])
      expect(safeHref(href)).toBe(href)
  })
  test('refuses what could run or leave disguised', () => {
    for (const href of [
      'javascript:alert(1)',
      'data:text/html,x',
      'vbscript:x',
    ])
      expect(safeHref(href)).toBeNull()
    expect(safeHref('//evil.example/x')).toBeNull()
  })
})

describe('inlineImage', () => {
  test('never loads an outside image by itself, which could carry data out', () => {
    expect(inlineImage('https://evil.example/x.png?secret=1')).toBeNull()
    expect(inlineImage('//evil.example/x.png')).toBeNull()
    expect(inlineImage('data:image/svg+xml;base64,AAAA')).toBeNull()
  })
  test('loads this console and encoded pictures', () => {
    expect(inlineImage('/api/attachments/a/b/content')).not.toBeNull()
    expect(inlineImage('data:image/png;base64,AAAA')).not.toBeNull()
  })
})

describe('local paths', () => {
  // What the browser itself would load, on the origin the console is at.
  const origin = (href: string) =>
    new URL(href, 'https://console.example').origin
  test('a path the browser reads as another host stays out', () => {
    const leaving = [
      '//evil.example/p',
      '/\\evil.example/p?d=S',
      '/\t/evil.example/p',
      '/\n/evil.example/p',
    ]
    for (const href of leaving) {
      expect(origin(href)).toBe('https://evil.example')
      expect(inlineImage(href)).toBeNull()
      expect(safeHref(href)).toBeNull()
    }
  })
})
