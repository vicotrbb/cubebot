import assert from 'node:assert/strict'
import { describe, test } from 'node:test'
import { day, isSelf, phrase, target } from './audit'

describe('audit sentences', () => {
  test('a kind reads as a verb phrase with its detail', () => {
    assert.equal(
      phrase('vault.fill', { field: 'secret' }),
      'filled the password of',
    )
    assert.equal(phrase('agent.lifecycle', { action: 'retire' }), 'retired')
    assert.equal(
      phrase('session.sign_in_failed', { login: 'admin' }),
      'failed to sign in as admin',
    )
    assert.equal(
      phrase('broadcast.send', { agents: 1 }),
      'broadcast to 1 agent',
    )
    assert.equal(
      phrase('desktop.take_control', { login: 'amazon.com.br' }),
      'took control to log in to amazon.com.br on',
    )
  })

  test('a person acting on their own account names no resource', () => {
    assert.equal(
      phrase('session.sign_in', { method: 'password' }, true),
      'signed in',
    )
    assert.equal(
      phrase('account.avatar', { removed: true }, true),
      'removed their profile picture',
    )
    assert.ok(isSelf('session.sign_out', 'a', 'a'))
    assert.ok(!isSelf('account.suspend', 'a', 'a'))
    assert.ok(!isSelf('session.sign_in', null, null))
  })

  test('an unknown kind still reads', () => {
    assert.equal(phrase('files.share_link', {}), 'files share link')
  })

  test('the resource is named by its current name, then the detail, then its id', () => {
    assert.equal(target('agent.create', {}, 'Scout', 'abc'), 'Scout')
    assert.equal(
      target('vault.delete', { name: 'GitHub' }, null, 'abc'),
      'GitHub',
    )
    assert.equal(target('room.create', {}, null, '0123456789'), '01234567…')
    assert.equal(target('broadcast.send', {}, 'x', 'y'), null)
  })

  test('days are relative near today', () => {
    const now = new Date(2026, 10, 3, 12)
    assert.equal(day(new Date(2026, 10, 3, 1).toISOString(), now), 'Today')
    assert.equal(day(new Date(2026, 10, 2, 23).toISOString(), now), 'Yesterday')
    assert.ok(day(new Date(2025, 0, 2).toISOString(), now).includes('2025'))
  })
})
