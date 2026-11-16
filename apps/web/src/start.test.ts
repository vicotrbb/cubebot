import assert from 'node:assert/strict'
import test from 'node:test'
import {
  frameAncestors,
  meetingsAnswer,
  meetingsHost,
  onMeetingsHost,
} from './start'

test('embedding is refused unless valid sources are configured', () => {
  assert.equal(frameAncestors(undefined), "'none'")
  assert.equal(frameAncestors('  '), "'none'")
  assert.equal(
    frameAncestors('https://ha.lan:8123 https://*.example.com'),
    'https://ha.lan:8123 https://*.example.com',
  )
  for (const unsafe of [
    "https://a.lan; script-src 'unsafe-inline'",
    '*',
    "'self' x",
  ])
    assert.equal(frameAncestors(unsafe), "'none'", unsafe)
})

test('the meetings host is guarded only when it differs from the console host', () => {
  assert.equal(meetingsHost(undefined, 'https://cubebot.lan'), undefined)
  assert.equal(
    meetingsHost('https://Cubebot.lan/', 'https://cubebot.lan'),
    undefined,
  )
  assert.equal(
    meetingsHost('https://Meetings.Example.com', 'https://cubebot.lan'),
    'meetings.example.com',
  )
  assert.equal(meetingsHost('https://meet.lan:8443'), 'meet.lan:8443')
})

const HOST = 'meetings.example.com'
const request = (path: string, headers: Record<string, string> = {}) =>
  new Request(`http://web:3000${path}`, { headers: { host: HOST, ...headers } })

test('requests on other hosts are untouched', () => {
  assert.equal(onMeetingsHost(request('/'), undefined), false)
  const console = new Request('http://cubebot.lan/meetings/x', {
    headers: { host: 'cubebot.lan' },
  })
  assert.equal(onMeetingsHost(console, HOST), false)
  for (const host of ['Meetings.Example.com:443', 'meetings.example.com.'])
    assert.equal(onMeetingsHost(request('/', { host }), HOST), true, host)
})

test('the meetings host serves only meeting pages and their static files', () => {
  for (const path of [
    '/m/abc',
    '/assets/index-1.js',
    '/fonts/inter.woff2',
    '/icons/x.svg',
    '/favicon.svg',
    '/manifest.webmanifest',
    '/call-capture.js',
    '/apple-touch-icon.png',
  ])
    assert.equal(meetingsAnswer(request(path), HOST), undefined, path)
  for (const path of [
    '/',
    '/meetings/x',
    '/x/m/abc',
    '/m',
    '/api/identity/me',
    '/sw.js',
  ])
    assert.equal(meetingsAnswer(request(path), HOST)?.status, 404, path)
})

test('plain HTTP on the meetings host is redirected to HTTPS', () => {
  const answer = meetingsAnswer(
    request('/m/abc?x=1', { 'x-forwarded-proto': 'http' }),
    HOST,
  )
  assert.equal(answer?.status, 308)
  assert.equal(
    answer?.headers.get('location'),
    'https://meetings.example.com/m/abc?x=1',
  )
  assert.equal(
    meetingsAnswer(request('/m/abc', { 'x-forwarded-proto': 'https' }), HOST),
    undefined,
  )
})
