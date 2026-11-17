import { Hand, MicOff, MonitorOff, MonitorUp, UserMinus, X } from 'lucide-react'
import { useEffect, useRef, useState, type ReactNode } from 'react'
import { api, ok } from '../../api/client'
import { Switch } from '../../components/primitives'
import type { Meeting } from './api'
import type { Engine } from './media/engine'
import { speakerName, type Participant, type Room } from './media/room'
import { displayName, Face } from './Tile'

export type PanelTab = 'people' | 'transcript'

function Waiting({ room, engine }: { room: Room; engine: Engine }) {
  if (!room.member || room.waiting.length === 0) return null
  return (
    <section className="meeting-panel__section" aria-label="Waiting to join">
      <h3>Waiting to join</h3>
      <ul className="meeting-people">
        {room.waiting.map((guest) => (
          <li key={guest.id} className="meeting-person">
            <span
              className="meeting-initial meeting-face meeting-face--small"
              aria-hidden="true"
            >
              {guest.name.charAt(0).toUpperCase()}
            </span>
            <span className="meeting-person__name">{guest.name}</span>
            <span className="meeting-person__actions">
              <button
                type="button"
                className="btn btn--sm btn--ghost"
                onClick={() => engine.send({ kind: 'deny', id: guest.id })}
              >
                Deny
              </button>
              <button
                type="button"
                className="btn btn--sm"
                onClick={() => engine.send({ kind: 'admit', id: guest.id })}
              >
                Admit
              </button>
            </span>
          </li>
        ))}
      </ul>
    </section>
  )
}

function ListenSwitch({
  participant,
  meeting,
  invitation,
}: {
  participant: Participant
  meeting: string
  invitation: string
}) {
  const [busy, setBusy] = useState(false)
  const [failed, setFailed] = useState(false)
  const change = async (listening: boolean) => {
    setBusy(true)
    setFailed(false)
    try {
      await ok(
        api.PATCH('/api/meetings/{id}/invitations/{invitation}', {
          params: { path: { id: meeting, invitation } },
          body: { listening },
        }),
      )
    } catch {
      setFailed(true)
    } finally {
      setBusy(false)
    }
  }
  return (
    <span className="meeting-person__listen">
      <Switch
        checked={Boolean(participant.listening)}
        disabled={busy}
        label={`Listen only, ${participant.name}`}
        onChange={(listening) => void change(listening)}
      />
      <span aria-live="polite">{failed ? 'Not saved' : 'Listen only'}</span>
    </span>
  )
}

function PersonActions({
  participant,
  room,
  engine,
  meeting,
}: {
  participant: Participant
  room: Room
  engine: Engine
  meeting?: Meeting | undefined
}) {
  const you = participant.slot === room.you
  const invitation = meeting?.invitations.find(
    (seat) => seat.agent && seat.agent === participant.agent,
  )
  return (
    <span className="meeting-person__actions">
      {room.host && meeting && invitation && (
        <ListenSwitch
          participant={participant}
          meeting={meeting.id}
          invitation={invitation.id}
        />
      )}
      {participant.hand && (
        <Hand className="meeting-person__mark" aria-label="Hand raised" />
      )}
      {!participant.mic && participant.kind !== 'agent' && (
        <MicOff className="meeting-person__mark" aria-label="Microphone off" />
      )}
      {participant.can_present && (
        <button
          type="button"
          className="btn btn--sm btn--ghost btn--icon"
          aria-label={
            participant.screen
              ? `Stop showing ${participant.name}'s screen`
              : `Show ${participant.name}'s screen`
          }
          title={participant.screen ? 'Stop showing screen' : 'Show screen'}
          onClick={() =>
            engine.send({
              kind: 'present',
              source: participant.slot,
              share: !participant.screen,
            })
          }
        >
          {participant.screen ? <MonitorOff /> : <MonitorUp />}
        </button>
      )}
      {room.host && !you && (
        <button
          type="button"
          className="btn btn--sm btn--ghost btn--icon"
          aria-label={`Remove ${participant.name}`}
          title="Remove from meeting"
          onClick={() =>
            engine.send({ kind: 'remove', source: participant.slot })
          }
        >
          <UserMinus />
        </button>
      )}
    </span>
  )
}

const order = (participant: Participant) =>
  participant.hand ? 0 : participant.kind === 'agent' ? 2 : 1

function People({
  room,
  engine,
  invite,
  meeting,
}: {
  room: Room
  engine: Engine
  invite?: ReactNode
  meeting?: Meeting | undefined
}) {
  const sorted = [...room.participants].sort(
    (a, b) => order(a) - order(b) || a.slot - b.slot,
  )
  return (
    <>
      {invite}
      <Waiting room={room} engine={engine} />
      <section className="meeting-panel__section" aria-label="In the meeting">
        <h3>In the meeting · {room.participants.length}</h3>
        <ul className="meeting-people">
          {sorted.map((participant) => (
            <li key={participant.slot} className="meeting-person">
              <Face participant={participant} size="small" />
              <span className="meeting-person__name">
                {displayName(participant, participant.slot === room.you)}
                {participant.host && <span className="chip">Host</span>}
                {participant.kind === 'agent' && (
                  <span className="chip">Agent</span>
                )}
              </span>
              <PersonActions
                participant={participant}
                room={room}
                engine={engine}
                meeting={meeting}
              />
            </li>
          ))}
        </ul>
      </section>
    </>
  )
}

const clockOf = (at: number) =>
  new Date(at).toLocaleTimeString(undefined, {
    hour: '2-digit',
    minute: '2-digit',
  })

function Transcript({ room }: { room: Room }) {
  const list = useRef<HTMLOListElement>(null)
  const pinned = useRef(true)
  useEffect(() => {
    const element = list.current
    if (element && pinned.current) element.scrollTop = element.scrollHeight
  }, [room.captions])
  if (room.captions.length === 0)
    return (
      <p className="meeting-panel__empty">
        What people and agents say appears here as they speak.
      </p>
    )
  return (
    <ol
      ref={list}
      className="meeting-transcript"
      aria-live="off"
      onScroll={(event) => {
        const element = event.currentTarget
        pinned.current =
          element.scrollHeight - element.scrollTop - element.clientHeight < 40
      }}
    >
      {room.captions.map((caption) => (
        <li key={caption.id} data-partial={!caption.final || undefined}>
          <span className="meeting-transcript__who">
            {speakerName(room, caption.source)}
            <time>{clockOf(caption.at)}</time>
          </span>
          <p>{caption.text}</p>
        </li>
      ))}
    </ol>
  )
}

export function Panel({
  tab,
  room,
  engine,
  invite,
  meeting,
  onTab,
  onClose,
}: {
  tab: PanelTab
  room: Room
  engine: Engine
  invite?: ReactNode
  meeting?: Meeting | undefined
  onTab: (tab: PanelTab) => void
  onClose: () => void
}) {
  return (
    <aside
      className="meeting-panel"
      aria-label={tab === 'people' ? 'People' : 'Transcript'}
    >
      <div className="meeting-panel__head">
        <div className="tabs">
          <button
            type="button"
            aria-pressed={tab === 'people'}
            onClick={() => onTab('people')}
          >
            People
            {room.waiting.length > 0 && room.member && (
              <span className="meeting-count">{room.waiting.length}</span>
            )}
          </button>
          <button
            type="button"
            aria-pressed={tab === 'transcript'}
            onClick={() => onTab('transcript')}
          >
            Transcript
          </button>
        </div>
        <button
          type="button"
          className="btn btn--ghost btn--icon"
          aria-label="Close panel"
          onClick={onClose}
        >
          <X />
        </button>
      </div>
      <div className="meeting-panel__body">
        {tab === 'people' ? (
          <People
            room={room}
            engine={engine}
            invite={invite}
            meeting={meeting}
          />
        ) : (
          <Transcript room={room} />
        )}
      </div>
    </aside>
  )
}
