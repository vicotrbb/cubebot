import { Hand, MicOff, MonitorUp } from 'lucide-react'
import { useCallback, useEffect, useRef } from 'react'
import { Cube, type Mood } from '../../components/Cube'
import { PersonAvatar } from '../../components/PersonAvatar'
import { initialOf } from './GreenRoom'
import type { Engine } from './media/engine'
import type { AgentState, Participant, Room } from './media/room'

const AGENT_SAYS: Record<AgentState, string> = {
  listening: 'Listening',
  thinking: 'Thinking…',
  speaking: 'Speaking',
}

export function agentMood(
  state: AgentState | undefined,
  sharing: boolean,
): Mood {
  if (sharing) return 'desktop'
  return state === 'thinking' ? 'working' : 'idle'
}

export function displayName(participant: Participant, you: boolean) {
  if (you) return `${participant.name} (you)`
  return participant.kind === 'guest'
    ? `${participant.name} (guest)`
    : participant.name
}

export function Face({
  participant,
  size,
}: {
  participant: Participant
  size: 'small' | 'large'
}) {
  if (participant.kind === 'agent')
    return (
      <Cube
        avatar={participant.avatar ?? null}
        size={size === 'large' ? 34 : 28}
      />
    )
  if (participant.account)
    return (
      <PersonAvatar
        account={participant.account}
        name={participant.name}
        className={`meeting-face meeting-face--${size}`}
      />
    )
  return (
    <span
      className={`meeting-initial meeting-face meeting-face--${size}`}
      aria-hidden="true"
    >
      {initialOf(participant.name)}
    </span>
  )
}

function SelfView({ stream }: { stream: MediaStream }) {
  const video = useRef<HTMLVideoElement>(null)
  useEffect(() => {
    if (video.current) video.current.srcObject = stream
  }, [stream])
  return (
    <video
      ref={video}
      className="tile__video tile__video--mirror"
      autoPlay
      playsInline
      muted
    />
  )
}

function RemoteView({ engine, slot }: { engine: Engine; slot: number }) {
  const attach = useCallback(
    (canvas: HTMLCanvasElement | null) =>
      engine.attachVideo(slot, 'camera', canvas),
    [engine, slot],
  )
  return <canvas ref={attach} className="tile__video" />
}

function Picture({
  participant,
  room,
  engine,
  you,
}: {
  participant: Participant
  room: Room
  engine: Engine
  you: boolean
}) {
  if (participant.kind === 'agent')
    return (
      <div className="tile__cube">
        <Cube
          avatar={participant.avatar ?? null}
          mood={agentMood(room.agents[participant.slot], participant.screen)}
          size={96}
          agentId={participant.agent}
        />
      </div>
    )
  const streaming = room.tracks[`${participant.slot}:camera`]?.codec
  if (participant.camera && you) return <SelfView stream={engine.local} />
  if (participant.camera && streaming)
    return <RemoteView engine={engine} slot={participant.slot} />
  if (participant.account)
    return (
      <PersonAvatar
        account={participant.account}
        name={participant.name}
        className="meeting-initial meeting-initial--large"
      />
    )
  return (
    <span className="meeting-initial meeting-initial--large" aria-hidden="true">
      {initialOf(participant.name)}
    </span>
  )
}

export function Tile({
  participant,
  room,
  engine,
  compact = false,
}: {
  participant: Participant
  room: Room
  engine: Engine
  compact?: boolean
}) {
  const you = participant.slot === room.you
  const agentState = room.agents[participant.slot]
  return (
    <figure
      className="tile"
      data-slot={participant.slot}
      data-kind={participant.kind}
      data-compact={compact || undefined}
      data-hand={participant.hand || undefined}
      aria-label={displayName(participant, you)}
    >
      <Picture
        participant={participant}
        room={room}
        engine={engine}
        you={you}
      />
      <figcaption className="tile__name">
        {!participant.mic && participant.kind !== 'agent' && (
          <MicOff className="tile__muted" aria-label="Microphone off" />
        )}
        <span>{displayName(participant, you)}</span>
        {participant.kind === 'agent' && !compact && (
          <span className="tile__state">
            {participant.screen
              ? 'Presenting'
              : AGENT_SAYS[agentState ?? 'listening']}
            {agentState === 'speaking' && !participant.screen && (
              <span className="tile__hint"> · talk to interrupt</span>
            )}
          </span>
        )}
      </figcaption>
      {participant.hand && (
        <span
          className="tile__badge tile__badge--hand"
          aria-label="Hand raised"
        >
          <Hand />
        </span>
      )}
      {participant.screen && participant.kind !== 'agent' && (
        <span className="tile__badge" aria-label="Presenting">
          <MonitorUp />
        </span>
      )}
    </figure>
  )
}
