import assert from 'node:assert/strict'
import { describe, test } from 'node:test'
import { headline, notifying, ruleTitle, scopeOf, severityOf } from './model'

describe('watchdog wording', () => {
  test('rules read as titles, unknown ones included', () => {
    assert.equal(ruleTitle('reverse_shell'), 'Reverse shell')
    assert.equal(ruleTitle('new_rule_kind'), 'New rule kind')
  })

  test('the head counts what is open and calls out high ones', () => {
    assert.equal(headline([]), 'nothing open')
    assert.equal(
      headline([{ severity: 'high' }, { severity: 'high' }]),
      '2 open',
    )
    assert.equal(
      headline([
        { severity: 'high' },
        { severity: 'low' },
        { severity: 'medium' },
      ]),
      '3 open · 1 high',
    )
  })

  test('scope, severity and the badge count', () => {
    assert.equal(scopeOf('Scout'), 'for Scout')
    assert.equal(scopeOf(null), 'everywhere')
    assert.equal(severityOf({ severity: 'unexpected' }), 'low')
    assert.equal(
      notifying([
        { kind: 'watchdog' },
        { kind: 'attention' },
        { kind: 'watchdog' },
      ]),
      2,
    )
    assert.equal(notifying(undefined), 0)
  })
})
