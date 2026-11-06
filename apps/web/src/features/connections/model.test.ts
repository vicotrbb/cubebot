import assert from 'node:assert/strict'
import test from 'node:test'
import {
  type Connection,
  type Manifest,
  grouped,
  matches,
  permitted,
  standing,
  suggestedSlug,
  value,
} from './model'

const tool = (
  name: string,
  risk: 'read' | 'write' | 'send',
  product?: string,
) => ({
  name,
  description: `${name} things`,
  risk,
  product: product ?? null,
})

const google = {
  id: 'google',
  name: 'Google',
  description: 'Gmail, Calendar and Drive.',
  category: 'mail_calendar',
  auth: [],
  config: [],
  products: [
    { id: 'gmail', name: 'Gmail' },
    { id: 'drive', name: 'Drive' },
  ],
  tools: [
    tool('gmail_search', 'read', 'gmail'),
    tool('gmail_send', 'send', 'gmail'),
    tool('drive_upload', 'write', 'drive'),
  ],
  events: [],
  level: 'full',
  guide: [],
} as unknown as Manifest

const connection = (patch: Partial<Connection>) =>
  ({
    status: 'connected',
    account: 'v@example.com',
    checked_at: null,
    ...patch,
  }) as Connection

test('a card reports the worst news first', () => {
  const ago = () => '2h ago'
  assert.deepEqual(standing([], ago), { tone: 'none', text: 'Not connected' })
  assert.equal(
    standing([connection({})], ago).text,
    'Connected as v@example.com',
  )
  assert.equal(
    standing([connection({}), connection({})], ago).text,
    '2 accounts',
  )
  const failing = connection({ status: 'error', checked_at: 'x' })
  assert.equal(
    standing([connection({}), failing], ago).text,
    'Error · checked 2h ago',
  )
  const again = connection({ status: 'sign_in_again' })
  assert.equal(standing([failing, again], ago).tone, 'again')
})

test('search needs every word, in names, products or tools', () => {
  assert.equal(matches(google, 'gmail send', null), true)
  assert.equal(matches(google, 'drive slack', null), false)
  assert.equal(matches(google, '', 'dev'), false)
})

test('levels permit what is at or below them', () => {
  assert.equal(permitted('read', 'write'), false)
  assert.equal(permitted('write', 'write'), true)
  assert.equal(permitted('write', 'send'), false)
  assert.equal(permitted('full', 'send'), true)
})

test('tools group by risk and skip products switched off', () => {
  const groups = grouped(google, ['gmail'])
  assert.deepEqual(
    groups.map((g) => g.risk),
    ['read', 'send'],
  )
  assert.equal(grouped(google, null).flatMap((g) => g.tools).length, 3)
})

test('names and values are typed for the server', () => {
  assert.equal(suggestedSlug(google, ['google', 'google-2']), 'google-3')
  const field = (type: string) => ({ kind: { type } }) as never
  assert.equal(value(field('number'), ' 993 '), 993)
  assert.deepEqual(value(field('lines'), '/v1\n\n /v2 '), ['/v1', '/v2'])
  assert.equal(value(field('text'), '  '), undefined)
})
