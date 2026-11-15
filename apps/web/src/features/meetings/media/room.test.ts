import test from 'node:test'
import assert from 'node:assert/strict'
import {
  dropped,
  EMPTY_ROOM,
  reduce,
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
