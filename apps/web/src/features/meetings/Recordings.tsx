import { Link } from '@tanstack/react-router'
import { CircleAlert, Download, FolderOpen, RotateCw } from 'lucide-react'
import { useState } from 'react'
import { api, failure, ok } from '../../api/client'
import { formatTime } from '../calendar/model'
import { bytes } from '../vitals/reading'
import { useRefresh, type Meeting } from './api'

type Recording = Meeting['recordings'][number]

function length(milliseconds: number) {
  const seconds = Math.round(milliseconds / 1000)
  const clock = `${Math.floor((seconds % 3600) / 60)}:${String(seconds % 60).padStart(2, '0')}`
  const hours = Math.floor(seconds / 3600)
  return hours ? `${hours}:${clock.padStart(5, '0')}` : clock
}

function facts(recording: Recording) {
  return [
    `Started by ${recording.started_by}`,
    recording.duration_ms != null && length(recording.duration_ms),
    recording.size != null && bytes(recording.size),
  ]
    .filter(Boolean)
    .join(' · ')
}

function Retry({
  meeting,
  recording,
}: {
  meeting: Meeting
  recording: Recording
}) {
  const refresh = useRefresh()
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  const retry = async () => {
    setBusy(true)
    setError('')
    try {
      await ok(
        api.POST('/api/meetings/{id}/recordings/{recording}/retry', {
          params: { path: { id: meeting.id, recording: recording.id } },
        }),
      )
      await refresh()
    } catch (caught) {
      setError(failure(caught))
    } finally {
      setBusy(false)
    }
  }
  return (
    <>
      <button
        type="button"
        className="btn btn--sm"
        disabled={busy}
        onClick={() => void retry()}
      >
        <RotateCw aria-hidden="true" /> {busy ? 'Retrying…' : 'Try again'}
      </button>
      {error && <span className="recording__error">{error}</span>}
    </>
  )
}

function Frame({
  meeting,
  recording,
}: {
  meeting: Meeting
  recording: Recording
}) {
  if (recording.state === 'ready' && recording.file)
    return (
      <video
        className="recording__video night"
        controls
        playsInline
        preload="metadata"
        src={`/api/files/${recording.file}/content`}
        aria-label={`Recording of ${meeting.title}`}
      />
    )
  if (recording.state === 'recording')
    return (
      <div className="recording__frame night" data-state="recording">
        <span className="meeting-rec">
          <span className="meeting-rec__dot" aria-hidden="true" />
          Recording now
        </span>
        <p>It is processed into a video as soon as recording stops.</p>
      </div>
    )
  if (recording.state === 'processing')
    return (
      <div
        className="recording__frame night"
        data-state="processing"
        role="status"
      >
        <strong className="tabular">Processing · {recording.progress}%</strong>
        <progress
          max={100}
          value={recording.progress}
          aria-label="Processing the recording"
        />
        <p>You will get a notification when it is ready.</p>
      </div>
    )
  if (recording.state === 'failed')
    return (
      <div className="recording__frame night" data-state="failed">
        <strong>
          <CircleAlert aria-hidden="true" /> The recording could not be
          processed
        </strong>
        {recording.error && <p>{recording.error}</p>}
        <Retry meeting={meeting} recording={recording} />
      </div>
    )
  return (
    <div className="recording__frame night" data-state="removed">
      <p>This recording was deleted from Files.</p>
    </div>
  )
}

function Item({
  meeting,
  recording,
  zone,
}: {
  meeting: Meeting
  recording: Recording
  zone: string
}) {
  const from = formatTime(recording.started_at, zone)
  const to = recording.stopped_at ? formatTime(recording.stopped_at, zone) : ''
  const ready = recording.state === 'ready' && recording.file
  return (
    <li className="recording">
      <Frame meeting={meeting} recording={recording} />
      <div className="recording__bar">
        <div className="recording__about">
          <span className="recording__time tabular">
            {to ? `${from} to ${to}` : `Since ${from}`}
          </span>
          <span className="faint">{facts(recording)}</span>
        </div>
        {ready && (
          <div className="recording__actions">
            <Link
              to="/files"
              className="btn btn--ghost btn--sm"
              title="Saved in Files, under Meeting recordings"
            >
              <FolderOpen aria-hidden="true" /> Files
            </Link>
            <a
              className="btn btn--sm"
              href={`/api/files/${recording.file}/content`}
              download
            >
              <Download aria-hidden="true" /> Download
            </a>
          </div>
        )}
      </div>
    </li>
  )
}

export function Recordings({
  meeting,
  zone,
}: {
  meeting: Meeting
  zone: string
}) {
  return (
    <ol className="recordings">
      {meeting.recordings.map((recording) => (
        <Item
          key={recording.id}
          meeting={meeting}
          recording={recording}
          zone={zone}
        />
      ))}
    </ol>
  )
}
