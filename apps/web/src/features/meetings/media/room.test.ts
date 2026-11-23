import test from 'node:test'
import assert from 'node:assert/strict'
import {
  dropped,
  EMPTY_ROOM,
  lost,
  reduce,
  RETRY_DELAYS,
  speakerName,
  stage,
  type Participant,
} from './room'

const person = (
  slot: number,
  extra: Partial<Participant> = {},
): Participant => ({
  slot,
  kind: 'person',
  name: `P${slot}`,
  host: false,
  mic: true,
  camera: false,
  screen: false,
  hand: false,
  ...extra,
})

const welcomed = reduce(EMPTY_ROOM, {
  kind: 'welcome',
  you: 1,
  title: 'Weekly',
  host: true,
  member: true,
  open_join: false,
  locked: false,
  can_record: true,
})

test('a guest waits, then is welcomed live', () => {
  const waiting = reduce(EMPTY_ROOM, { kind: 'waiting' })
  assert.equal(waiting.phase, 'waiting')
  assert.equal(welcomed.phase, 'live')
  assert.equal(welcomed.you, 1)
})

test('a partial caption is replaced by its final line', () => {
  const partial = reduce(welcomed, {
    kind: 'caption',
    source: 2,
    id: 'u1',
    text: 'hel',
    final: false,
  })
  const final = reduce(partial, {
    kind: 'caption',
    source: 2,
    id: 'u1',
    text: 'hello Ada',
    final: true,
  })
  assert.equal(final.captions.length, 1)
  assert.equal(final.captions[0]?.text, 'hello Ada')
  assert.equal(final.captions[0]?.final, true)
})

test('the latest presenter takes the stage and leaving gives it back', () => {
  const one = reduce(welcomed, {
    kind: 'roster',
    participants: [person(1), person(2, { screen: true })],
  })
  const two = reduce(one, {
    kind: 'roster',
    participants: [
      person(1),
      person(2, { screen: true }),
      person(3, { kind: 'agent', screen: true }),
    ],
  })
  assert.equal(stage(two), 3)
  const back = reduce(two, {
    kind: 'roster',
    participants: [person(1), person(2, { screen: true })],
  })
  assert.equal(stage(back), 2)
})

test('a lost socket drops a live room but keeps a final outcome', () => {
  assert.equal(dropped(welcomed).phase, 'dropped')
  assert.equal(dropped(reduce(welcomed, { kind: 'ended' })).phase, 'ended')
})

test('speakers are named for the viewer', () => {
  const room = reduce(welcomed, {
    kind: 'roster',
    participants: [person(1), person(4, { kind: 'guest', name: 'Rita' })],
  })
  assert.equal(speakerName(room, 1), 'You')
  assert.equal(speakerName(room, 4), 'Rita (guest)')
  assert.equal(speakerName(room, 9), 'Someone')
})

test('an unexplained close mid-meeting retries, then gives up', () => {
  let room = reduce(welcomed, { kind: 'notice', text: 'Old news' })
  for (let tries = 1; tries <= RETRY_DELAYS.length; tries += 1) {
    room = lost(room, '')
    assert.equal(room.phase, 'reconnecting')
    assert.equal(room.retries, tries)
    assert.equal(room.notice, '', 'a stale notice does not come back')
  }
  assert.equal(lost(room, '').phase, 'dropped')
})

test('a close the server explains is final and shows why', () => {
  const removed = lost(welcomed, 'The host removed you from the meeting.')
  assert.equal(removed.phase, 'dropped')
  assert.equal(removed.notice, 'The host removed you from the meeting.')
  assert.equal(
    lost(lost(welcomed, ''), 'The meeting is full.').phase,
    'dropped',
  )
  assert.equal(lost(reduce(welcomed, { kind: 'ended' }), '').phase, 'ended')
})

test('only a room already joined reconnects by itself', () => {
  assert.equal(lost(EMPTY_ROOM, '').phase, 'dropped')
  const waiting = reduce(EMPTY_ROOM, { kind: 'waiting' })
  assert.equal(lost(waiting, '').phase, 'dropped')
})

test('a welcome back ends the retries and drops tracks of the old connection', () => {
  const tracked = reduce(welcomed, {
    kind: 'track',
    source: 2,
    stream: 'camera',
    codec: 'vp8',
    width: 640,
    height: 360,
  })
  const back = reduce(lost(lost(tracked, ''), ''), {
    kind: 'welcome',
    you: 7,
    title: 'Weekly',
    host: false,
    member: false,
    open_join: false,
    locked: false,
    can_record: false,
    rejoin: 'next',
  })
  assert.equal(back.phase, 'live')
  assert.equal(back.retries, 0)
  assert.equal(back.you, 7)
  assert.deepEqual(back.tracks, {})
})

test('the retries span the time the server keeps a seat to come back to', () => {
  const total = RETRY_DELAYS.reduce((sum, delay) => sum + delay, 0)
  assert.ok(total <= 60_000 && total >= 30_000)
})

test('everyone learns who started and stopped recording, and a rejoin starts from the server', () => {
  assert.equal(welcomed.canRecord, true)
  const on = reduce(welcomed, { kind: 'recording', on: true, by: 'Ana' })
  assert.deepEqual(on.recording, { by: 'Ana' })
  assert.match(on.notice, /^Ana started recording/)
  assert.equal(reduce(on, { kind: 'recording', on: true, by: 'Ana' }), on)
  const off = reduce(on, { kind: 'recording', on: false, by: 'Bo' })
  assert.equal(off.recording, null)
  assert.match(off.notice, /^Bo stopped recording/)
  const lost = reduce(on, { kind: 'recording', on: false })
  assert.equal(lost.recording, null)
  assert.equal(lost.notice, on.notice)
  const back = reduce(on, {
    kind: 'welcome',
    you: 4,
    title: 'Weekly',
    host: false,
    member: false,
    open_join: false,
    locked: false,
    can_record: false,
  })
  assert.equal(back.recording, null)
  assert.equal(back.canRecord, false)
})
