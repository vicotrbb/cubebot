import { useCallback, useEffect, useRef, useState } from 'react'
import type { Devices } from './media/engine'

const VOICE = {
  channelCount: 1,
  echoCancellation: true,
  noiseSuppression: true,
  autoGainControl: true,
}
const PICTURE = {
  width: { ideal: 1280 },
  height: { ideal: 720 },
  frameRate: { ideal: 24 },
}

const audioWith = (device?: string) =>
  device ? { ...VOICE, deviceId: { exact: device } } : VOICE
const videoWith = (device?: string) =>
  device ? { ...PICTURE, deviceId: { exact: device } } : PICTURE

export function mediaProblem(error: unknown, what: string) {
  if (!globalThis.isSecureContext)
    return `Meetings need the ${what}, which browsers only allow over HTTPS or on localhost.`
  if (error instanceof DOMException && error.name === 'NotAllowedError')
    return `The ${what} is blocked. Allow it for this site in your browser settings.`
  if (error instanceof DOMException && error.name === 'NotFoundError')
    return `No ${what} was found on this device.`
  if (error instanceof DOMException && error.name === 'NotReadableError')
    return `The ${what} is in use by another app.`
  return `The ${what} could not be started.`
}

function replace(
  stream: MediaStream,
  kind: 'audio' | 'video',
  next: MediaStream,
) {
  for (const track of kind === 'audio'
    ? stream.getAudioTracks()
    : stream.getVideoTracks()) {
    track.stop()
    stream.removeTrack(track)
  }
  for (const track of next.getTracks()) stream.addTrack(track)
}

async function grab(kind: 'audio' | 'video', device?: string) {
  return navigator.mediaDevices.getUserMedia(
    kind === 'audio'
      ? { audio: audioWith(device) }
      : { video: videoWith(device) },
  )
}

export function usePreview() {
  const [stream] = useState(() => new MediaStream())
  const [mic, setMicState] = useState(true)
  const [camera, setCameraState] = useState(true)
  const [chosen, setChosen] = useState<Devices>({})
  const [inputs, setInputs] = useState<MediaDeviceInfo[]>([])
  const [problems, setProblems] = useState<{ audio?: string; video?: string }>(
    {},
  )
  const [starting, setStarting] = useState(true)
  const handed = useRef(false)

  const start = useCallback(
    async (kind: 'audio' | 'video', device?: string) => {
      try {
        replace(stream, kind, await grab(kind, device))
        setProblems((current) => ({ ...current, [kind]: undefined }))
        return true
      } catch (error) {
        const what = kind === 'audio' ? 'microphone' : 'camera'
        setProblems((current) => ({
          ...current,
          [kind]: mediaProblem(error, what),
        }))
        return false
      }
    },
    [stream],
  )

  useEffect(() => {
    let live = true
    void (async () => {
      const heard = await start('audio')
      const seen = await start('video')
      if (!live) return
      setMicState(heard)
      setCameraState(seen)
      setStarting(false)
      const listed = await navigator.mediaDevices
        .enumerateDevices()
        .catch(() => [])
      if (live)
        setInputs(
          listed.filter(
            (device) => device.kind !== 'audiooutput' && device.deviceId,
          ),
        )
    })()
    return () => {
      live = false
      if (!handed.current) for (const track of stream.getTracks()) track.stop()
    }
  }, [start, stream])

  const setMic = (on: boolean) => {
    for (const track of stream.getAudioTracks()) track.enabled = on
    setMicState(on && stream.getAudioTracks().length > 0)
  }

  const setCamera = async (on: boolean) => {
    if (!on) {
      replace(stream, 'video', new MediaStream())
      setCameraState(false)
      return
    }
    setCameraState(await start('video', chosen.camera))
  }

  const choose = async (kind: 'audio' | 'video', device: string) => {
    setChosen((current) => ({
      ...current,
      [kind === 'audio' ? 'microphone' : 'camera']: device,
    }))
    if (kind === 'video' && !camera) return
    const ok = await start(kind, device)
    if (kind === 'audio' && ok)
      for (const track of stream.getAudioTracks()) track.enabled = mic
  }

  return {
    stream,
    mic,
    camera,
    chosen,
    microphones: inputs.filter((device) => device.kind === 'audioinput'),
    cameras: inputs.filter((device) => device.kind === 'videoinput'),
    problems,
    starting,
    setMic,
    setCamera,
    choose,
    hand: () => {
      handed.current = true
    },
  }
}

export type Preview = ReturnType<typeof usePreview>
