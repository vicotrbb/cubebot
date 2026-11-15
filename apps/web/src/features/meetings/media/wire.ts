export type Stream = 'camera' | 'screen'

export const AUDIO = 1
export const CAMERA = 2
export const SCREEN = 3

const KINDS: Record<Stream, number> = { camera: CAMERA, screen: SCREEN }

export type Incoming =
  | { kind: 'audio'; source: number; rate: number; samples: Float32Array }
  | {
      kind: 'video'
      source: number
      stream: Stream
      key: boolean
      timestamp: number
      data: Uint8Array
    }

export function microphoneFrame(pcm: ArrayBuffer) {
  const frame = new Uint8Array(pcm.byteLength + 1)
  frame[0] = AUDIO
  frame.set(new Uint8Array(pcm), 1)
  return frame.buffer
}

export function videoFrame(
  stream: Stream,
  key: boolean,
  timestamp: number,
  chunk: Uint8Array,
) {
  const frame = new Uint8Array(chunk.byteLength + 6)
  const view = new DataView(frame.buffer)
  view.setUint8(0, KINDS[stream])
  view.setUint8(1, key ? 1 : 0)
  view.setUint32(2, timestamp >>> 0, true)
  frame.set(chunk, 6)
  return frame.buffer
}

function pcm(bytes: ArrayBuffer, offset: number) {
  const count = Math.floor((bytes.byteLength - offset) / 2)
  const view = new DataView(bytes, offset, count * 2)
  const samples = new Float32Array(count)
  for (let index = 0; index < count; index += 1)
    samples[index] = view.getInt16(index * 2, true) / 32768
  return samples
}

export function incoming(bytes: ArrayBuffer): Incoming | null {
  if (bytes.byteLength < 3) return null
  const view = new DataView(bytes)
  const type = view.getUint8(0)
  const source = view.getUint16(1, true)
  if (type === AUDIO) {
    if (bytes.byteLength < 7) return null
    const rate = view.getUint32(3, true)
    return rate ? { kind: 'audio', source, rate, samples: pcm(bytes, 7) } : null
  }
  if ((type !== CAMERA && type !== SCREEN) || bytes.byteLength < 8) return null
  return {
    kind: 'video',
    source,
    stream: type === CAMERA ? 'camera' : 'screen',
    key: (view.getUint8(3) & 1) === 1,
    timestamp: view.getUint32(4, true),
    data: new Uint8Array(bytes, 8),
  }
}
