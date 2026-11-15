import { videoFrame, type Stream } from './wire'

export type Shape = {
  width: number
  height: number
  fps: number
  bitrate: number
}

export const SHAPES: Record<Stream, Shape> = {
  camera: { width: 640, height: 360, fps: 24, bitrate: 500_000 },
  screen: { width: 1920, height: 1080, fps: 8, bitrate: 1_500_000 },
}

const CODECS = ['vp8', 'avc1.42E01F', 'vp09.00.10.08']
const KEY_EVERY = 4000
const CONGESTED = 1_000_000

export type Announce = (
  codec: string | null,
  width: number,
  height: number,
) => void

export const videoSupported = () =>
  typeof VideoEncoder !== 'undefined' && typeof VideoFrame !== 'undefined'

const even = (value: number) => Math.max(2, Math.floor(value / 2) * 2)

export function fitted(width: number, height: number, shape: Shape) {
  const scale = Math.min(1, shape.width / width, shape.height / height)
  return { width: even(width * scale), height: even(height * scale) }
}

function encoderConfig(
  codec: string,
  width: number,
  height: number,
  shape: Shape,
) {
  return {
    codec,
    width,
    height,
    bitrate: shape.bitrate,
    framerate: shape.fps,
    latencyMode: 'realtime' as const,
    ...(codec.startsWith('avc1') ? { avc: { format: 'annexb' as const } } : {}),
  }
}

async function supportedCodec(width: number, height: number, shape: Shape) {
  for (const codec of CODECS) {
    const answer = await VideoEncoder.isConfigSupported(
      encoderConfig(codec, width, height, shape),
    ).catch(() => null)
    if (answer?.supported) return codec
  }
  return null
}

function hiddenVideo(track: MediaStreamTrack) {
  const element = document.createElement('video')
  element.muted = true
  element.playsInline = true
  element.srcObject = new MediaStream([track])
  void element.play().catch(() => undefined)
  return element
}

export function encode(
  stream: Stream,
  track: MediaStreamTrack,
  send: (frame: ArrayBuffer) => void,
  announce: Announce,
  backlog: () => number,
) {
  const shape = SHAPES[stream]
  const video = hiddenVideo(track)
  const canvas = document.createElement('canvas')
  const context = canvas.getContext('2d', { alpha: false })
  let encoder: VideoEncoder | null = null
  let lastKey = 0
  let wantKey = true
  let configuring = false
  let stopped = false
  const configure = async (width: number, height: number) => {
    configuring = true
    const codec = await supportedCodec(width, height, shape)
    configuring = false
    if (stopped) return
    encoder?.close()
    encoder = null
    canvas.width = width
    canvas.height = height
    if (!codec) {
      announce(null, width, height)
      return
    }
    encoder = new VideoEncoder({
      output: (chunk) => {
        const data = new Uint8Array(chunk.byteLength)
        chunk.copyTo(data)
        send(
          videoFrame(
            stream,
            chunk.type === 'key',
            Math.round(chunk.timestamp / 1000),
            data,
          ),
        )
      },
      error: () => {
        encoder = null
        canvas.width = 0
      },
    })
    encoder.configure(encoderConfig(codec, width, height, shape))
    wantKey = true
    announce(codec, width, height)
  }
  const tick = () => {
    if (stopped || configuring || !context || video.readyState < 2) return
    const size = fitted(video.videoWidth, video.videoHeight, shape)
    if (size.width !== canvas.width || size.height !== canvas.height) {
      void configure(size.width, size.height)
      return
    }
    if (!encoder || encoder.encodeQueueSize > 2 || backlog() > CONGESTED) return
    context.drawImage(video, 0, 0, canvas.width, canvas.height)
    const now = performance.now()
    const keyFrame = wantKey || now - lastKey > KEY_EVERY
    const frame = new VideoFrame(canvas, { timestamp: Math.round(now * 1000) })
    encoder.encode(frame, { keyFrame })
    frame.close()
    if (keyFrame) {
      lastKey = now
      wantKey = false
    }
  }
  const timer = setInterval(tick, 1000 / shape.fps)
  return {
    keyframe() {
      wantKey = true
    },
    stop() {
      stopped = true
      clearInterval(timer)
      if (encoder?.state === 'configured') encoder.close()
      video.srcObject = null
      announce(null, 0, 0)
    },
  }
}

export type Encoding = ReturnType<typeof encode>

type Picture = {
  decoder: VideoDecoder | null
  canvas: HTMLCanvasElement | null
  waitingForKey: boolean
  codec: string | null
}

export function pictures(
  askKeyframe: (source: number, stream: Stream) => void,
) {
  const all = new Map<string, Picture>()
  const named = (source: number, stream: Stream) => `${source}:${stream}`
  const picture = (key: string) => {
    const found = all.get(key)
    if (found) return found
    const made: Picture = {
      decoder: null,
      canvas: null,
      waitingForKey: true,
      codec: null,
    }
    all.set(key, made)
    return made
  }
  const close = (target: Picture) => {
    if (target.decoder && target.decoder.state !== 'closed')
      target.decoder.close()
    target.decoder = null
  }
  const draw = (target: Picture, frame: VideoFrame) => {
    const canvas = target.canvas
    if (canvas) {
      if (canvas.width !== frame.displayWidth) canvas.width = frame.displayWidth
      if (canvas.height !== frame.displayHeight)
        canvas.height = frame.displayHeight
      canvas.getContext('2d')?.drawImage(frame, 0, 0)
    }
    frame.close()
  }
  const start = (
    source: number,
    stream: Stream,
    target: Picture,
    codec: string,
  ) => {
    if (typeof VideoDecoder === 'undefined') return
    const decoder = new VideoDecoder({
      output: (frame) => draw(target, frame),
      error: () => {
        target.decoder = null
        target.waitingForKey = true
        askKeyframe(source, stream)
      },
    })
    try {
      decoder.configure({ codec, optimizeForLatency: true })
    } catch {
      decoder.close()
      target.codec = null
      return
    }
    target.decoder = decoder
    target.waitingForKey = true
  }
  return {
    track(source: number, stream: Stream, codec: string | null) {
      const target = picture(named(source, stream))
      close(target)
      target.codec = codec
      if (codec) start(source, stream, target, codec)
    },
    chunk(
      source: number,
      stream: Stream,
      key: boolean,
      timestamp: number,
      data: Uint8Array,
    ) {
      const target = all.get(named(source, stream))
      if (!target?.codec) return
      if (!target.decoder) start(source, stream, target, target.codec)
      const decoder = target.decoder
      if (!decoder) return
      if (target.waitingForKey && !key) return
      target.waitingForKey = false
      decoder.decode(
        new EncodedVideoChunk({
          type: key ? 'key' : 'delta',
          timestamp: timestamp * 1000,
          data,
        }),
      )
    },
    attach(source: number, stream: Stream, canvas: HTMLCanvasElement | null) {
      picture(named(source, stream)).canvas = canvas
      if (canvas) askKeyframe(source, stream)
    },
    forget(source: number) {
      for (const stream of ['camera', 'screen'] as const) {
        const target = all.get(named(source, stream))
        if (target) close(target)
        all.delete(named(source, stream))
      }
    },
    close() {
      for (const target of all.values()) close(target)
      all.clear()
    },
  }
}

export type Pictures = ReturnType<typeof pictures>
