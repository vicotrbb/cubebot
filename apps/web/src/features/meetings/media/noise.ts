export type Noise = 'off' | 'standard' | 'strong'

export const NOISE_LEVELS: [Noise, string, string][] = [
  ['off', 'Off', 'Your microphone exactly as it hears the room.'],
  ['standard', 'Standard', 'Your browser filters steady hum and hiss.'],
  [
    'strong',
    'Strong',
    'Also removes typing, fans, traffic and other sharp background noise.',
  ],
]

const KEY = 'cubebot.meetings.noise'
const RNNOISE_RATE = 48000

export function savedNoise(): Noise {
  try {
    const saved = localStorage.getItem(KEY)
    return NOISE_LEVELS.some(([level]) => level === saved)
      ? (saved as Noise)
      : 'strong'
  } catch {
    return 'strong'
  }
}

export function saveNoise(noise: Noise) {
  try {
    localStorage.setItem(KEY, noise)
  } catch {
    return
  }
}

export function captureContext() {
  try {
    return new AudioContext({ sampleRate: RNNOISE_RATE })
  } catch {
    return new AudioContext()
  }
}

let binary: Promise<ArrayBuffer> | null = null

export async function suppressor(context: AudioContext) {
  if (context.sampleRate !== RNNOISE_RATE) return null
  const [library, worklet, wasm, simd] = await Promise.all([
    import('@sapphi-red/web-noise-suppressor'),
    import('@sapphi-red/web-noise-suppressor/rnnoiseWorklet.js?url'),
    import('@sapphi-red/web-noise-suppressor/rnnoise.wasm?url'),
    import('@sapphi-red/web-noise-suppressor/rnnoise_simd.wasm?url'),
  ])
  binary ??= library
    .loadRnnoise({ url: wasm.default, simdUrl: simd.default })
    .catch((error: unknown) => {
      binary = null
      throw error
    })
  await context.audioWorklet.addModule(worklet.default)
  return new library.RnnoiseWorkletNode(context, {
    maxChannels: 1,
    wasmBinary: await binary,
  })
}

export function browserFilter(track: MediaStreamTrack, noise: Noise) {
  return track
    .applyConstraints({
      channelCount: 1,
      echoCancellation: true,
      autoGainControl: true,
      noiseSuppression: noise !== 'off',
    })
    .catch(() => undefined)
}
