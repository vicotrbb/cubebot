import assert from 'node:assert/strict'
import { describe, test } from 'node:test'
import { AUDIT_COLUMNS, auditCsv, csv } from './csv'

describe('csv', () => {
  test('quotes fields with commas, quotes and line breaks', () => {
    assert.equal(
      csv([['plain', 'a,b', 'say "hi"', 'two\nlines', null, 7]]),
      'plain,"a,b","say ""hi""","two\nlines",,7\r\n',
    )
  })

  test('defuses spreadsheet formulas', () => {
    assert.equal(csv([['=SUM(A1)', '@cmd', 'ok']]), "'=SUM(A1),'@cmd,ok\r\n")
  })

  test('writes objects as JSON', () => {
    assert.equal(csv([[{ a: 1 }]]), '"{""a"":1}"\r\n')
  })
})

describe('audit export', () => {
  test('one header row, then one row per entry with its sentence', () => {
    const text = auditCsv([
      {
        sequence: 12,
        at: '2026-11-27T10:00:00Z',
        kind: 'account.suspend',
        actor: 'a1',
        actor_name: 'victor',
        resource: 'b2',
        resource_name: 'sam',
        detail: {},
      },
    ])
    const [header, row, end] = text.split('\r\n')
    assert.equal(header, AUDIT_COLUMNS.join(','))
    assert.equal(
      row,
      '12,2026-11-27T10:00:00Z,victor,a1,,,account.suspend,suspended,sam,b2,',
    )
    assert.equal(end, '')
  })
})
