import test from 'node:test'
import assert from 'node:assert/strict'
import { incoming, microphoneFrame, videoFrame } from './wire'

function relayed(type: number, source: number, body: Uint8Array) {
  const frame = new Uint8Array(body.byteLength + 3)
  frame[0] = type
  new DataView(frame.buffer).setUint16(1, source, true)
  frame.set(body, 3)
  return frame.buffer
}

test('a microphone frame is its PCM behind the audio type', () => {
  const pcm = new Int16Array([1, -2, 3]).buffer
  const frame = new Uint8Array(microphoneFrame(pcm))
  assert.equal(frame[0], 1)
  assert.equal(frame.byteLength, 7)
})

test('a video frame relayed with its source reads back whole', () => {
  const sent = new Uint8Array(
    videoFrame('screen', true, 70_000, new Uint8Array([9, 8, 7])),
  )
  const back = incoming(relayed(sent[0] ?? 0, 513, sent.subarray(1)))
  assert.deepEqual(back, {
    kind: 'video',
    source: 513,
    stream: 'screen',
    key: true,
    timestamp: 70_000,
    data: new Uint8Array([9, 8, 7]),
  })
})

test('audio carries its rate and becomes floats', () => {
  const body = new Uint8Array(4 + 4)
  const view = new DataView(body.buffer)
  view.setUint32(0, 24_000, true)
  view.setInt16(4, 16384, true)
  view.setInt16(6, -32768, true)
  const back = incoming(relayed(1, 2, body))
  assert.equal(back?.kind, 'audio')
  if (back?.kind !== 'audio') return
  assert.equal(back.rate, 24_000)
  assert.deepEqual([...back.samples], [0.5, -1])
})

test('short or unknown frames are ignored', () => {
  assert.equal(incoming(new ArrayBuffer(2)), null)
  assert.equal(incoming(relayed(1, 1, new Uint8Array(2))), null)
  assert.equal(incoming(relayed(9, 1, new Uint8Array(10))), null)
  assert.equal(incoming(relayed(1, 1, new Uint8Array(4))), null)
})
