import { microphoneFrame } from './wire'

const LEAD = 0.08
const BEHIND = 0.6
const QUIET_AFTER = 350

type Voice = {
  next: number
  playing: Set<AudioBufferSourceNode>
  level: number
  heard: number
}

function loudness(samples: Float32Array) {
  let sum = 0
  for (const sample of samples) sum += sample * sample
  return Math.min(1, Math.sqrt(sum / Math.max(1, samples.length)) * 4)
}

export function speakers(context: AudioContext) {
  const output = context.createGain()
  output.connect(context.destination)
  const voices = new Map<number, Voice>()
  const voice = (source: number) => {
    const found = voices.get(source)
    if (found) return found
    const made = {
      next: 0,
      playing: new Set<AudioBufferSourceNode>(),
      level: 0,
      heard: 0,
    }
    voices.set(source, made)
    return made
  }
  const stop = (source: number) => {
    const found = voices.get(source)
    if (!found) return
    for (const node of found.playing) {
      node.onended = null
      node.stop()
    }
    found.playing.clear()
    found.next = 0
  }
  return {
    play(source: number, rate: number, samples: Float32Array) {
      if (samples.length === 0) return
      const target = voice(source)
      const now = context.currentTime
      if (target.next - now > BEHIND) stop(source)
      const buffer = context.createBuffer(1, samples.length, rate)
      buffer.getChannelData(0).set(samples)
      const node = context.createBufferSource()
      node.buffer = buffer
      node.connect(output)
      node.onended = () => target.playing.delete(node)
      target.next = Math.max(target.next, now + LEAD)
      node.start(target.next)
      target.next += buffer.duration
      target.playing.add(node)
      target.level = loudness(samples)
      target.heard = performance.now()
    },
    stop,
    level(source: number) {
      const found = voices.get(source)
      if (!found || performance.now() - found.heard > QUIET_AFTER) return 0
      return found.level
    },
    forget(source: number) {
      stop(source)
      voices.delete(source)
    },
    close() {
      for (const source of voices.keys()) stop(source)
      output.disconnect()
    },
  }
}

export type Speakers = ReturnType<typeof speakers>

export async function microphone(
  context: AudioContext,
  stream: MediaStream,
  send: (frame: ArrayBuffer) => void,
  open: () => boolean,
) {
  await context.audioWorklet.addModule('/call-capture.js')
  const source = context.createMediaStreamSource(stream)
  const level = context.createAnalyser()
  level.fftSize = 512
  const worklet = new AudioWorkletNode(context, 'call-capture')
  worklet.port.onmessage = (event: MessageEvent<ArrayBuffer>) => {
    if (open()) send(microphoneFrame(event.data))
  }
  const silent = context.createGain()
  silent.gain.value = 0
  source.connect(level)
  source.connect(worklet)
  worklet.connect(silent).connect(context.destination)
  const samples = new Uint8Array(256)
  return {
    level() {
      level.getByteTimeDomainData(samples)
      return (
        samples.reduce(
          (peak, value) => Math.max(peak, Math.abs(value - 128)),
          0,
        ) / 128
      )
    },
    close() {
      worklet.port.onmessage = null
      source.disconnect()
      worklet.disconnect()
      silent.disconnect()
    },
  }
}

export type Microphone = Awaited<ReturnType<typeof microphone>>
