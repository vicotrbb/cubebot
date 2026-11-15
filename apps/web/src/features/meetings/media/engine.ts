import { HEIGHT, WIDTH, paint } from '../../desktop/useDesktopSocket'
import { microphone, speakers, type Microphone } from './audio'
import type { Server } from './room'
import { encode, pictures, videoSupported, type Encoding } from './video'
import { incoming, type Stream } from './wire'

export type Self = {
  mic: boolean
  camera: boolean
  screen: boolean
  hand: boolean
}

export type Devices = {
  microphone?: string | undefined
  camera?: string | undefined
}

export type Setup = {
  url: string
  name?: string | undefined
  context: AudioContext
  local: MediaStream
  devices: Devices
  self: Self
  onServer: (message: Server) => void
  onSelf: (self: Self) => void
  onClosed: (reason: string) => void
}

const PING_EVERY = 10_000

export const screenSupported = () =>
  typeof navigator !== 'undefined' &&
  typeof navigator.mediaDevices?.getDisplayMedia === 'function' &&
  videoSupported()

export function stopTracks(
  stream: MediaStream | null,
  kind?: 'audio' | 'video',
) {
  for (const track of stream?.getTracks() ?? [])
    if (!kind || track.kind === kind) {
      track.stop()
      stream?.removeTrack(track)
    }
}

export function connect(setup: Setup) {
  const socket = new WebSocket(setup.url)
  socket.binaryType = 'arraybuffer'
  const voices = speakers(setup.context)
  const encoders = new Map<Stream, Encoding>()
  const desktops = new Map<number, HTMLCanvasElement>()
  let self = { ...setup.self }
  let mic: Microphone | null = null
  let screen: MediaStream | null = null
  let live = false
  let closed = false

  const json = (message: Record<string, unknown>) => {
    if (socket.readyState === WebSocket.OPEN)
      socket.send(JSON.stringify(message))
  }
  const binary = (frame: ArrayBuffer) => {
    if (socket.readyState === WebSocket.OPEN) socket.send(frame)
  }
  const views = pictures((source, stream) =>
    json({ kind: 'want_keyframe', source, stream }),
  )
  const announce =
    (stream: Stream) => (codec: string | null, width: number, height: number) =>
      json({ kind: 'track', stream, codec, width, height })
  const changed = (next: Partial<Self>) => {
    self = { ...self, ...next }
    setup.onSelf(self)
    json({ kind: 'state', ...self })
  }

  const startEncoder = (
    stream: Stream,
    track: MediaStreamTrack | undefined,
  ) => {
    encoders.get(stream)?.stop()
    encoders.delete(stream)
    if (!track || !videoSupported()) return
    encoders.set(
      stream,
      encode(
        stream,
        track,
        binary,
        announce(stream),
        () => socket.bufferedAmount,
      ),
    )
  }

  const startMedia = async () => {
    if (live || closed) return
    live = true
    changed({})
    if (self.camera) startEncoder('camera', setup.local.getVideoTracks()[0])
    const started = await microphone(
      setup.context,
      setup.local,
      binary,
      () => self.mic,
    ).catch(() => null)
    if (closed) started?.close()
    else mic = started
  }

  const effects = (message: Server) => {
    if (message.kind === 'welcome') void startMedia()
    if (message.kind === 'keyframe') encoders.get(message.stream)?.keyframe()
    if (message.kind === 'track')
      views.track(message.source, message.stream, message.codec)
    if (message.kind === 'interrupt') voices.stop(message.source)
    if (message.kind === 'roster')
      forgetGone(message.participants.map((one) => one.slot))
    if (message.kind === 'desktop' && message.tiles) {
      const canvas = desktop(message.source).getContext('2d')
      if (canvas) void paint(canvas, message.tiles)
    }
  }

  const desktop = (source: number) => {
    const found = desktops.get(source)
    if (found) return found
    const made = document.createElement('canvas')
    made.width = WIDTH
    made.height = HEIGHT
    made.className = 'meeting-desktop'
    desktops.set(source, made)
    return made
  }

  const known = new Set<number>()
  const forgetGone = (slots: number[]) => {
    const present = new Set(slots)
    for (const slot of known)
      if (!present.has(slot)) {
        voices.forget(slot)
        views.forget(slot)
        desktops.get(slot)?.remove()
        desktops.delete(slot)
        known.delete(slot)
      }
    for (const slot of slots) known.add(slot)
  }

  socket.onopen = () => json({ kind: 'hello', name: setup.name })
  socket.onmessage = (event: MessageEvent<ArrayBuffer | string>) => {
    if (typeof event.data === 'string') {
      const message = JSON.parse(event.data) as Server
      effects(message)
      setup.onServer(message)
      return
    }
    const frame = incoming(event.data)
    if (frame?.kind === 'audio')
      voices.play(frame.source, frame.rate, frame.samples)
    if (frame?.kind === 'video')
      views.chunk(
        frame.source,
        frame.stream,
        frame.key,
        frame.timestamp,
        frame.data,
      )
  }
  socket.onclose = (event) => {
    if (!closed) setup.onClosed(event.reason)
  }
  const ping = setInterval(() => json({ kind: 'ping' }), PING_EVERY)

  const cameraOn = async () => {
    if (!setup.local.getVideoTracks().length) {
      const got = await navigator.mediaDevices.getUserMedia({
        video: setup.devices.camera
          ? { deviceId: { exact: setup.devices.camera } }
          : true,
      })
      for (const track of got.getVideoTracks()) setup.local.addTrack(track)
    }
    startEncoder('camera', setup.local.getVideoTracks()[0])
    changed({ camera: true })
  }

  const stopScreen = () => {
    startEncoder('screen', undefined)
    stopTracks(screen)
    screen = null
    changed({ screen: false })
  }

  return {
    local: setup.local,
    setMic(on: boolean) {
      for (const track of setup.local.getAudioTracks()) track.enabled = on
      changed({ mic: on })
    },
    async setCamera(on: boolean) {
      if (on) return cameraOn()
      startEncoder('camera', undefined)
      stopTracks(setup.local, 'video')
      changed({ camera: false })
    },
    async startScreen() {
      const shared = await navigator.mediaDevices.getDisplayMedia({
        video: true,
        audio: false,
      })
      const track = shared.getVideoTracks()[0]
      if (!track) return
      stopScreen()
      screen = shared
      track.addEventListener('ended', stopScreen)
      startEncoder('screen', track)
      changed({ screen: true })
    },
    stopScreen,
    setHand(on: boolean) {
      changed({ hand: on })
    },
    send: json,
    attachVideo: views.attach,
    attachDesktop(source: number, container: HTMLElement | null) {
      if (container) container.replaceChildren(desktop(source))
    },
    level(source: number, you: boolean) {
      return you ? (self.mic ? (mic?.level() ?? 0) : 0) : voices.level(source)
    },
    close() {
      closed = true
      clearInterval(ping)
      mic?.close()
      for (const encoding of encoders.values()) encoding.stop()
      stopTracks(screen)
      views.close()
      voices.close()
      socket.close()
    },
  }
}

export type Engine = ReturnType<typeof connect>
