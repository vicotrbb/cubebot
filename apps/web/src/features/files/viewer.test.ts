// @ts-expect-error bun provides this at test time
import { describe, expect, test } from 'bun:test'
import {
  blobType,
  fileSize,
  grammarOf,
  kindOf,
  parseTable,
  shownName,
} from './viewer'

describe('kindOf', () => {
  test('goes by the name first, since agents upload without a type', () => {
    expect(kindOf('chart.PNG')).toBe('image')
    expect(kindOf('voice.mp3', 'application/octet-stream')).toBe('audio')
    expect(kindOf('notes.md')).toBe('markdown')
    expect(kindOf('Dockerfile')).toBe('text')
    expect(kindOf('report.pdf')).toBe('pdf')
  })
  test('falls back on the type and refuses what it cannot show', () => {
    expect(kindOf('blob', 'image/webp')).toBe('image')
    expect(kindOf('blob', 'text/plain; charset=utf-8')).toBe('text')
    expect(kindOf('archive.zip', 'application/zip')).toBeNull()
    expect(kindOf('data.bin')).toBeNull()
  })
  test('a name like an inherited property is only a name', () => {
    for (const name of ['a.constructor', 'a.__proto__', 'toString']) {
      expect(kindOf(name)).toBeNull()
      expect(blobType(name)).toBe('text/plain')
      expect(grammarOf(name)).toBeUndefined()
    }
  })
})

describe('blobType', () => {
  test('never lets a blob be something that runs in the origin', () => {
    expect(blobType('page.html', 'text/html')).toBe('text/plain')
    expect(blobType('run.js')).toBe('text/plain')
    expect(blobType('evil.txt', 'text/html')).toBe('text/plain')
    expect(blobType('evil', 'text/html')).toBe('text/plain')
  })
  test('types media from its name so a mislabelled upload still plays', () => {
    expect(blobType('a.mp3', 'application/octet-stream')).toBe('audio/mpeg')
    expect(blobType('a.svg')).toBe('image/svg+xml')
    expect(blobType('a.pdf')).toBe('application/pdf')
  })
})

test('parseTable keeps quoted separators, doubled quotes and line endings', () => {
  expect(parseTable('a,b\r\n"x,1","say ""hi"""\n')).toEqual([
    ['a', 'b'],
    ['x,1', 'say "hi"'],
  ])
  expect(parseTable('a,b')).toEqual([['a', 'b']])
  expect(parseTable('a,\n,b')).toEqual([
    ['a', ''],
    ['', 'b'],
  ])
  expect(parseTable('')).toEqual([])
})

test('shownName drops only the tag an agent send adds', () => {
  expect(shownName('chart-1f3a9c2e.png')).toBe('chart.png')
  expect(shownName('audio-1f3a9c2e')).toBe('audio')
  expect(shownName('my-report.png')).toBe('my-report.png')
})

test('fileSize', () => {
  expect(fileSize(469)).toBe('469 B')
  expect(fileSize(2048)).toBe('2.0 KiB')
})
