import { useEffect, useReducer, useRef, useState } from 'react'
import { decodeAudio, phase, reduce, START, type Message } from './call'

function microphoneProblem(error: unknown) {
  if (!globalThis.isSecureContext)
    return 'Calls need the microphone, which browsers only allow over HTTPS or on localhost.'
  if (error instanceof DOMException && error.name === 'NotAllowedError')
    return 'The microphone is blocked. Allow it for this site in your browser, then call again.'
  if (error instanceof DOMException && error.name === 'NotFoundError')
    return 'No microphone was found on this device.'
  return 'The microphone could not be started.'
}

function player(context: AudioContext) {
  const output = context.createAnalyser()
  output.fftSize = 512
  output.connect(context.destination)
  const playing = new Set<AudioBufferSourceNode>()
  let next = 0
  return {
    output,
    playing,
    play(frame: ArrayBuffer, onChange: () => void) {
      const { rate, samples } = decodeAudio(frame)
      if (samples.length === 0) return
      const buffer = context.createBuffer(1, samples.length, rate)
      buffer.getChannelData(0).set(samples)
      const source = context.createBufferSource()
      source.buffer = buffer
      source.connect(output)
      source.onended = () => {
        playing.delete(source)
        onChange()
      }
      next = Math.max(next, context.currentTime + 0.02)
      source.start(next)
      next += buffer.duration
      playing.add(source)
      onChange()
    },
    stop() {
      for (const source of playing) {
        source.onended = null
        source.stop()
      }
      playing.clear()
      next = 0
    },
  }
}

async function capture(
  context: AudioContext,
  socket: WebSocket,
  level: AnalyserNode,
) {
  const stream = await navigator.mediaDevices.getUserMedia({
    audio: {
      channelCount: 1,
      echoCancellation: true,
      noiseSuppression: true,
      autoGainControl: true,
    },
  })
  const stopTracks = () => {
    for (const track of stream.getTracks()) track.stop()
  }
  try {
    await context.audioWorklet.addModule('/call-capture.js')
    const source = context.createMediaStreamSource(stream)
    const worklet = new AudioWorkletNode(context, 'call-capture')
    worklet.port.onmessage = (event: MessageEvent<ArrayBuffer>) => {
      if (socket.readyState === WebSocket.OPEN) socket.send(event.data)
    }
    source.connect(level)
    source.connect(worklet)
    // Some browsers only run a node that reaches the destination; the silent gain keeps it from playing.
    const silent = context.createGain()
    silent.gain.value = 0
    worklet.connect(silent).connect(context.destination)
    return () => {
      worklet.port.onmessage = null
      source.disconnect()
      worklet.disconnect()
      silent.disconnect()
      stopTracks()
    }
  } catch (error) {
    // Nothing else holds the stream yet, and a live track keeps the microphone on.
    stopTracks()
    throw error
  }
}

function keepAwake() {
  const lock = navigator.wakeLock?.request('screen').catch(() => null)
  return () => void lock?.then((held) => held?.release())
}

/** `context` comes from the click that placed the call, since mobile browsers only start audio from a gesture. */
export function useCall(
  agentId: string,
  context: AudioContext,
  stage: React.RefObject<HTMLElement | null>,
) {
  const [state, dispatch] = useReducer(reduce, START)
  const [playing, setPlaying] = useState(false)
  const [ended, setEnded] = useState('')
  const [hearing, setHearing] = useState(false)
  const started = useRef(Date.now())
  useEffect(() => {
    const scheme = location.protocol === 'https:' ? 'wss' : 'ws'
    const socket = new WebSocket(
      `${scheme}://${location.host}/api/agents/${agentId}/call`,
    )
    socket.binaryType = 'arraybuffer'
    const speaker = player(context)
    const microphone = context.createAnalyser()
    microphone.fftSize = 512
    const changed = () => setPlaying(speaker.playing.size > 0)
    let stopCapture = () => {}
    let closed = false
    socket.onmessage = (event: MessageEvent<ArrayBuffer | string>) => {
      if (typeof event.data !== 'string') {
        speaker.play(event.data, changed)
        return
      }
      const message = JSON.parse(event.data) as Message
      if (message.kind === 'interrupt') {
        speaker.stop()
        changed()
      }
      if (message.kind === 'ready') started.current = Date.now()
      dispatch(message)
    }
    socket.onopen = () => {
      capture(context, socket, microphone).then(
        (stop) => {
          if (closed) stop()
          else {
            stopCapture = stop
            setHearing(true)
          }
        },
        (error: unknown) => {
          setEnded(microphoneProblem(error))
          socket.close()
        },
      )
    }
    socket.onclose = (event) => {
      if (!closed)
        setEnded(
          (current) =>
            current ||
            event.reason ||
            'The call dropped. Check your connection and call again.',
        )
    }
    const release = keepAwake()
    const samples = new Uint8Array(256)
    let frame = requestAnimationFrame(function tick() {
      const source = speaker.playing.size > 0 ? speaker.output : microphone
      source.getByteTimeDomainData(samples)
      const peak = samples.reduce(
        (highest, value) => Math.max(highest, Math.abs(value - 128)),
        0,
      )
      stage.current?.style.setProperty('--level', String(peak / 128))
      frame = requestAnimationFrame(tick)
    })
    return () => {
      closed = true
      cancelAnimationFrame(frame)
      release()
      stopCapture()
      speaker.stop()
      socket.close()
    }
  }, [agentId, context, stage])
  return {
    state,
    phase: hearing ? phase(state, playing) : ('connecting' as const),
    hearing,
    ended,
    started,
  }
}
