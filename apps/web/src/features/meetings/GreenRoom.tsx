import { AudioLines, Mic, MicOff, Video, VideoOff } from 'lucide-react'
import { useEffect, useRef, useState, type ReactNode } from 'react'
import { Cube } from '../../components/Cube'
import { Alert, Field } from '../../components/primitives'
import type { Joining } from './useMeeting'
import { usePreview, type Preview } from './usePreview'

const NAME_KEY = 'cubebot.meetings.name'

function remembered() {
  try {
    return localStorage.getItem(NAME_KEY) ?? ''
  } catch {
    return ''
  }
}

function remember(name: string) {
  try {
    localStorage.setItem(NAME_KEY, name)
  } catch {
    return
  }
}

export const initialOf = (name: string) =>
  name.trim().charAt(0).toUpperCase() || '?'

function useLevel(
  preview: Preview,
  meter: React.RefObject<HTMLElement | null>,
) {
  useEffect(() => {
    const track = preview.stream.getAudioTracks()[0]
    if (!track || !preview.mic) {
      meter.current?.style.setProperty('--level', '0')
      return
    }
    const context = new AudioContext()
    void context.resume()
    const source = context.createMediaStreamSource(new MediaStream([track]))
    const analyser = context.createAnalyser()
    analyser.fftSize = 512
    source.connect(analyser)
    const samples = new Uint8Array(256)
    let frame = requestAnimationFrame(function tick() {
      analyser.getByteTimeDomainData(samples)
      const peak = samples.reduce(
        (top, value) => Math.max(top, Math.abs(value - 128)),
        0,
      )
      meter.current?.style.setProperty(
        '--level',
        String(Math.min(1, (peak / 128) * 2.2)),
      )
      frame = requestAnimationFrame(tick)
    })
    return () => {
      cancelAnimationFrame(frame)
      void context.close()
    }
  }, [meter, preview.mic, preview.stream, preview.starting])
}

function Mirror({ preview, name }: { preview: Preview; name: string }) {
  const video = useRef<HTMLVideoElement>(null)
  useEffect(() => {
    if (video.current)
      video.current.srcObject = preview.camera ? preview.stream : null
  }, [preview.camera, preview.stream, preview.chosen.camera])
  return (
    <div className="green-room__mirror">
      {preview.camera ? (
        <video
          ref={video}
          autoPlay
          playsInline
          muted
          aria-label="Your camera"
        />
      ) : (
        <div className="green-room__off">
          <span
            className="meeting-initial meeting-initial--large"
            aria-hidden="true"
          >
            {initialOf(name)}
          </span>
          <p>
            {preview.starting ? 'Starting your camera…' : 'Your camera is off'}
          </p>
        </div>
      )}
    </div>
  )
}

function Toggles({
  preview,
  meter,
}: {
  preview: Preview
  meter: React.RefObject<HTMLSpanElement | null>
}) {
  return (
    <div className="green-room__toggles">
      <button
        type="button"
        className="meeting-control"
        aria-pressed={!preview.mic}
        data-off={!preview.mic || undefined}
        aria-label={preview.mic ? 'Turn off microphone' : 'Turn on microphone'}
        disabled={!preview.stream.getAudioTracks().length}
        onClick={() => preview.setMic(!preview.mic)}
      >
        {preview.mic ? <Mic /> : <MicOff />}
      </button>
      <button
        type="button"
        className="meeting-control"
        aria-pressed={!preview.camera}
        data-off={!preview.camera || undefined}
        aria-label={preview.camera ? 'Turn off camera' : 'Turn on camera'}
        onClick={() => void preview.setCamera(!preview.camera)}
      >
        {preview.camera ? <Video /> : <VideoOff />}
      </button>
      <span ref={meter} className="green-room__meter" aria-hidden="true">
        <AudioLines />
      </span>
    </div>
  )
}

function DevicePicker({
  label,
  devices,
  value,
  onChange,
}: {
  label: string
  devices: MediaDeviceInfo[]
  value: string | undefined
  onChange: (device: string) => void
}) {
  if (devices.length < 2) return null
  return (
    <Field label={label}>
      <select
        className="input"
        value={value ?? ''}
        onChange={(event) => onChange(event.target.value)}
      >
        {!value && <option value="">Default</option>}
        {devices.map((device, index) => (
          <option key={device.deviceId} value={device.deviceId}>
            {device.label || `${label} ${index + 1}`}
          </option>
        ))}
      </select>
    </Field>
  )
}

export function GreenRoom({
  title,
  detail,
  guest,
  url,
  joinLabel = 'Join now',
  aside,
  onBack,
  onJoin,
}: {
  title: string
  detail?: ReactNode
  guest: boolean
  url: string
  joinLabel?: string
  aside?: ReactNode
  onBack?: () => void
  onJoin: (joining: Joining) => void
}) {
  const preview = usePreview()
  const [name, setName] = useState(remembered)
  const meter = useRef<HTMLSpanElement>(null)
  useLevel(preview, meter)
  const trimmed = name.trim()
  const ready = !preview.starting && (!guest || trimmed.length > 0)
  const join = () => {
    if (!ready) return
    const context = new AudioContext()
    void context.resume()
    if (guest) remember(trimmed)
    preview.hand()
    onJoin({
      url,
      name: guest ? trimmed : undefined,
      context,
      local: preview.stream,
      devices: preview.chosen,
      self: {
        mic: preview.mic,
        camera: preview.camera,
        screen: false,
        hand: false,
      },
    })
  }
  return (
    <div className="green-room night">
      {guest && (
        <div className="green-room__brand">
          <Cube size={20} />
          Cubebot
        </div>
      )}
      <div className="green-room__frame">
        <section
          className="green-room__preview"
          aria-label="Your camera and microphone"
        >
          <Mirror preview={preview} name={guest ? trimmed : title} />
          <Toggles preview={preview} meter={meter} />
        </section>
        <form
          className="green-room__details"
          onSubmit={(event) => {
            event.preventDefault()
            join()
          }}
        >
          <div className="green-room__head">
            <h1>{title}</h1>
            {detail && <div className="green-room__detail">{detail}</div>}
          </div>
          {aside}
          {guest && (
            <Field label="Your name" hint="Shown to everyone">
              <input
                className="input"
                value={name}
                maxLength={40}
                autoComplete="name"
                required
                onChange={(event) => setName(event.target.value)}
              />
            </Field>
          )}
          <DevicePicker
            label="Microphone"
            devices={preview.microphones}
            value={preview.chosen.microphone}
            onChange={(device) => void preview.choose('audio', device)}
          />
          <DevicePicker
            label="Camera"
            devices={preview.cameras}
            value={preview.chosen.camera}
            onChange={(device) => void preview.choose('video', device)}
          />
          <Alert>{preview.problems.audio}</Alert>
          {preview.problems.video && (
            <p className="notice">{preview.problems.video}</p>
          )}
          <p className="green-room__notice">
            This meeting is transcribed, and agents in it hear what you say.
          </p>
          <div className="green-room__actions">
            {onBack && (
              <button type="button" className="btn btn--ghost" onClick={onBack}>
                Back
              </button>
            )}
            <button
              type="submit"
              className="btn btn--primary green-room__join"
              disabled={!ready}
            >
              {joinLabel}
            </button>
          </div>
        </form>
      </div>
    </div>
  )
}
