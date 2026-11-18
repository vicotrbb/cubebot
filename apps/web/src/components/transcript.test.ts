import test from 'node:test'
import assert from 'node:assert/strict'
import { fileName, json, markdown, type Sheet } from './transcript'

const sheet: Sheet = {
  kind: 'meeting',
  title: 'Daily Tesar 18/11: Ação',
  started_at: '2026-11-18T16:16:24Z',
  ended_at: '2026-11-18T16:29:36Z',
  details: [['Host', 'Victor Bona']],
  summary: '## Decisions\n- Ship it\n',
  lines: [
    {
      speaker: 'Victor Bona',
      kind: 'person',
      at: '2026-11-18T16:16:30Z',
      text: 'Bom dia',
    },
    { speaker: 'Ayla', kind: 'agent', at: null, text: 'Hello' },
  ],
}

test('file names are plain ASCII slugs with the day', () => {
  assert.equal(fileName(sheet, 'md'), 'daily-tesar-18-11-acao-2026-11-18.md')
  assert.equal(
    fileName({ ...sheet, title: '***', started_at: null }, 'pdf'),
    'meeting.pdf',
  )
})

test('markdown carries details, summary and every line in order', () => {
  assert.equal(
    markdown(sheet, () => '16:16'),
    [
      '# Daily Tesar 18/11: Ação',
      '- **Host:** Victor Bona',
      '## Summary\n\n### Decisions\n- Ship it',
      '## Transcript\n\n**Victor Bona** · 16:16\nBom dia\n\n**Ayla**\nHello\n',
    ].join('\n\n'),
  )
})

test('markdown says so when nothing was said', () => {
  assert.match(
    markdown({ ...sheet, summary: null, lines: [] }, String),
    /## Transcript\n\nNothing was said\.\n$/,
  )
})

test('json keeps machine fields and names the transcript', () => {
  const parsed = JSON.parse(json(sheet))
  assert.deepEqual(parsed.details, { Host: 'Victor Bona' })
  assert.equal(parsed.transcript.length, 2)
  assert.equal(parsed.started_at, '2026-11-18T16:16:24Z')
  assert.equal(parsed.lines, undefined)
})
