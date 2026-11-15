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
    assert.equal(
      phrase('oidc.configure', {
        issuer: 'https://id.example.com',
        secret_changed: true,
      }),
      'set up single sign-on with https://id.example.com and a new client secret',
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

  test('meeting invitations are counted without addresses', () => {
    assert.equal(
      phrase('meeting.invite', {
        name: 'Planning',
        accounts: 1,
        agents: 2,
        emails: 0,
      }),
      'invited 1 member, 2 agents to the meeting',
    )
    assert.equal(
      target('meeting.invite', { name: 'Planning' }, null, 'abc'),
      'Planning',
    )
    assert.equal(target('meeting.configure', {}, null, null), null)
  })

  test('meeting room actions name the guest or participant', () => {
    assert.equal(
      phrase('meeting.guest_join', { name: 'Planning', guest: 'Dee' }),
      'joined as the guest Dee in',
    )
    assert.equal(
      phrase('meeting.deny', { guest: 'Dee' }),
      'turned away the guest Dee from',
    )
    assert.equal(
      phrase('meeting.remove', { participant: 'Bob' }),
      'removed Bob from',
    )
    assert.equal(
      phrase('meeting.lock', { locked: false }),
      'unlocked the meeting',
    )
    assert.equal(
      target('meeting.admit', { name: 'Planning', guest: 'Dee' }, null, 'abc'),
      'Planning',
    )
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

test('plan sign-ins name the plan and an unconfirmed revocation', () => {
  assert.equal(
    phrase('provider.sign_in', { provider: 'supergrok' }),
    'signed in their Grok plan',
  )
  assert.equal(
    phrase('provider.sign_out', { provider: 'chatgpt', revoked: false }),
    'signed out the ChatGPT plan, without the vendor confirming',
  )
  assert.equal(target('provider.sign_out', {}, 'x', 'y'), null)
  assert.equal(
    phrase('provider.sign_in', { provider: 'minimax_plan_cn' }),
    'signed in their MiniMax plan',
  )
})
