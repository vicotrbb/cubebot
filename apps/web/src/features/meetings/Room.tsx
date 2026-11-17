import {
  Captions,
  Ellipsis,
  Hand,
  Lock,
  LockOpen,
  Mic,
  MicOff,
  MonitorUp,
  MonitorX,
  PhoneOff,
  ScrollText,
  Users,
  Video,
  VideoOff,
} from 'lucide-react'
import { useCallback, useEffect, useRef, useState, type ReactNode } from 'react'
import { Cube, type Mood } from '../../components/Cube'
import { Menu } from '../../components/primitives'
import { bestGrid } from './grid'
import { screenSupported, type Engine } from './media/engine'
import {
  stage,
  speakerName,
  type Participant,
  type Phase,
  type Room as RoomState,
} from './media/room'
import type { Meeting } from './api'
import { Panel, type PanelTab } from './Panel'
import { Tile } from './Tile'
import { useMeeting, type Joining } from './useMeeting'
import './meetings.css'

const SPEAKING = 0.12
const CAPTION_FOR = 7000
const GAP = 10

function useLevels(
  container: React.RefObject<HTMLElement | null>,
  engine: React.RefObject<Engine | null>,
  you: number | null,
) {
  useEffect(() => {
    let frame = requestAnimationFrame(function tick() {
      const current = engine.current
      for (const tile of container.current?.querySelectorAll<HTMLElement>(
        '[data-slot]',
      ) ?? []) {
        const slot = Number(tile.dataset.slot)
        const level = current?.level(slot, slot === you) ?? 0
        tile.style.setProperty('--level', level.toFixed(3))
        tile.toggleAttribute('data-speaking', level > SPEAKING)
      }
      frame = requestAnimationFrame(tick)
    })
    return () => cancelAnimationFrame(frame)
  }, [container, engine, you])
}

function useSize(element: React.RefObject<HTMLElement | null>) {
  const [size, setSize] = useState({ width: 0, height: 0 })
  useEffect(() => {
    const target = element.current
    if (!target) return
    const observer = new ResizeObserver(([entry]) => {
      if (entry)
        setSize({
          width: entry.contentRect.width,
          height: entry.contentRect.height,
        })
    })
    observer.observe(target)
    return () => observer.disconnect()
  }, [element])
  return size
}

function Elapsed() {
  const [start] = useState(Date.now)
  const [now, setNow] = useState(start)
  useEffect(() => {
    const timer = setInterval(() => setNow(Date.now()), 1000)
    return () => clearInterval(timer)
  }, [])
  const seconds = Math.floor((now - start) / 1000)
  const hours = Math.floor(seconds / 3600)
  const clock = `${Math.floor((seconds % 3600) / 60)}:${String(seconds % 60).padStart(2, '0')}`
  return (
    <time className="meeting-top__clock">
      {hours ? `${hours}:${clock.padStart(5, '0')}` : clock}
    </time>
  )
}

function Grid({
  room,
  engine,
  people,
}: {
  room: RoomState
  engine: Engine
  people: Participant[]
}) {
  const area = useRef<HTMLDivElement>(null)
  const size = useSize(area)
  const ratio = size.height > size.width ? 3 / 4 : 16 / 9
  const grid = bestGrid(people.length, size.width, size.height, GAP, ratio)
  return (
    <div ref={area} className="meeting-grid">
      <div
        className="meeting-grid__tiles"
        style={
          {
            '--tile-w': `${grid.width}px`,
            '--tile-ratio': ratio,
            '--columns': grid.columns,
          } as React.CSSProperties
        }
      >
        {people.map((participant) => (
          <Tile
            key={participant.slot}
            participant={participant}
            room={room}
            engine={engine}
          />
        ))}
      </div>
    </div>
  )
}

function Screen({
  presenter,
  engine,
  you,
}: {
  presenter: Participant
  engine: Engine
  you: boolean
}) {
  const desktop = useCallback(
    (element: HTMLDivElement | null) =>
      engine.attachDesktop(presenter.slot, element),
    [engine, presenter.slot],
  )
  const canvas = useCallback(
    (element: HTMLCanvasElement | null) =>
      engine.attachVideo(presenter.slot, 'screen', element),
    [engine, presenter.slot],
  )
  if (you)
    return (
      <div className="meeting-screen__self">
        <MonitorUp />
        <p>You are presenting to everyone</p>
        <button type="button" className="btn" onClick={engine.stopScreen}>
          Stop presenting
        </button>
      </div>
    )
  if (presenter.kind === 'agent')
    return <div ref={desktop} className="meeting-screen__desktop" />
  return <canvas ref={canvas} className="meeting-screen__video" />
}

function Stage({
  room,
  engine,
  presenter,
}: {
  room: RoomState
  engine: Engine
  presenter: Participant
}) {
  const you = presenter.slot === room.you
  return (
    <div className="meeting-stage">
      <figure
        className="meeting-screen"
        aria-label={`${presenter.name}'s screen`}
      >
        <Screen presenter={presenter} engine={engine} you={you} />
        {!you && (
          <figcaption className="meeting-screen__label">
            <MonitorUp />
            {presenter.kind === 'agent'
              ? `${presenter.name} is sharing their desktop`
              : `${presenter.name} is presenting`}
          </figcaption>
        )}
      </figure>
      <div className="meeting-strip" aria-label="Participants">
        {room.participants.map((participant) => (
          <Tile
            key={participant.slot}
            participant={participant}
            room={room}
            engine={engine}
            compact
          />
        ))}
      </div>
    </div>
  )
}

function CaptionBand({ room }: { room: RoomState }) {
  const [now, setNow] = useState(Date.now)
  useEffect(() => {
    const timer = setInterval(() => setNow(Date.now()), 1000)
    return () => clearInterval(timer)
  }, [])
  const recent = room.captions
    .filter((caption) => now - caption.at < CAPTION_FOR)
    .slice(-2)
  if (recent.length === 0) return null
  return (
    <div className="meeting-captions" aria-live="polite">
      {recent.map((caption) => (
        <p key={caption.id} data-partial={!caption.final || undefined}>
          <b>{speakerName(room, caption.source)}</b> {caption.text}
        </p>
      ))}
    </div>
  )
}

function Toggle({
  on,
  label,
  offLabel = label,
  danger = false,
  hidden = false,
  shortcut,
  children,
  onClick,
}: {
  on: boolean
  label: string
  offLabel?: string
  danger?: boolean
  hidden?: boolean
  shortcut?: string
  children: ReactNode
  onClick: () => void
}) {
  if (hidden) return null
  return (
    <button
      type="button"
      className="meeting-control"
      aria-pressed={on}
      data-off={(danger && !on) || undefined}
      data-active={(!danger && on) || undefined}
      aria-label={on ? label : offLabel}
      title={
        shortcut
          ? `${on ? label : offLabel} (${shortcut})`
          : on
            ? label
            : offLabel
      }
      aria-keyshortcuts={shortcut}
      onClick={onClick}
    >
      {children}
    </button>
  )
}

type Controls = {
  room: RoomState
  engine: Engine
  self: ReturnType<typeof useMeeting>['self']
  captions: boolean
  panel: PanelTab | null
  setCaptions: (on: boolean) => void
  setPanel: (tab: PanelTab | null) => void
  onLeave: () => void
}

function More({
  room,
  engine,
  captions,
  setCaptions,
  setPanel,
  self,
}: Controls) {
  return (
    <Menu
      className="meeting-more"
      triggerClassName="meeting-control"
      label="More options"
      trigger={<Ellipsis />}
    >
      <button
        type="button"
        className="menu__item meeting-more__phone"
        onClick={() => setCaptions(!captions)}
      >
        <Captions /> {captions ? 'Hide captions' : 'Show captions'}
      </button>
      <button
        type="button"
        className="menu__item meeting-more__phone"
        onClick={() => engine.setHand(!self.hand)}
      >
        <Hand /> {self.hand ? 'Lower hand' : 'Raise hand'}
      </button>
      <button
        type="button"
        className="menu__item meeting-more__phone"
        onClick={() => setPanel('transcript')}
      >
        <ScrollText /> Transcript
      </button>
      {room.host && (
        <button
          type="button"
          className="menu__item"
          onClick={() => engine.send({ kind: 'lock', locked: !room.locked })}
        >
          {room.locked ? <LockOpen /> : <Lock />}{' '}
          {room.locked ? 'Unlock meeting' : 'Lock meeting'}
        </button>
      )}
      {room.host && (
        <button
          type="button"
          className="menu__item menu__item--danger"
          onClick={() => engine.send({ kind: 'end' })}
        >
          <PhoneOff /> End for everyone
        </button>
      )}
    </Menu>
  )
}

function Bar(controls: Controls) {
  const {
    room,
    engine,
    self,
    captions,
    panel,
    setCaptions,
    setPanel,
    onLeave,
  } = controls
  const share = () =>
    self.screen
      ? engine.stopScreen()
      : void engine.startScreen().catch(() => undefined)
  return (
    <footer className="meeting-bar">
      <div className="meeting-bar__title">
        <Elapsed />
        <span>{room.title}</span>
      </div>
      <div
        className="meeting-bar__controls"
        role="toolbar"
        aria-label="Meeting controls"
      >
        <Toggle
          on={self.mic}
          danger
          label="Turn off microphone"
          offLabel="Turn on microphone"
          shortcut="M"
          onClick={() => engine.setMic(!self.mic)}
        >
          {self.mic ? <Mic /> : <MicOff />}
        </Toggle>
        <Toggle
          on={self.camera}
          danger
          label="Turn off camera"
          offLabel="Turn on camera"
          shortcut="V"
          onClick={() =>
            void engine.setCamera(!self.camera).catch(() => undefined)
          }
        >
          {self.camera ? <Video /> : <VideoOff />}
        </Toggle>
        <Toggle
          on={self.screen}
          label="Stop presenting"
          offLabel="Present your screen"
          hidden={!screenSupported()}
          onClick={share}
        >
          {self.screen ? <MonitorX /> : <MonitorUp />}
        </Toggle>
        <span className="meeting-bar__wide">
          <Toggle
            on={captions}
            label="Hide captions"
            offLabel="Show captions"
            shortcut="C"
            onClick={() => setCaptions(!captions)}
          >
            <Captions />
          </Toggle>
          <Toggle
            on={self.hand}
            label="Lower hand"
            offLabel="Raise hand"
            shortcut="H"
            onClick={() => engine.setHand(!self.hand)}
          >
            <Hand />
          </Toggle>
        </span>
        <More {...controls} />
        <button
          type="button"
          className="meeting-leave"
          onClick={onLeave}
          aria-label="Leave meeting"
        >
          <PhoneOff />
          <span>Leave</span>
        </button>
      </div>
      <div className="meeting-bar__panels">
        <Toggle
          on={panel === 'people'}
          label="Hide people"
          offLabel="Show people"
          onClick={() => setPanel(panel === 'people' ? null : 'people')}
        >
          <Users />
          {room.member && room.waiting.length > 0 && (
            <span className="meeting-count">{room.waiting.length}</span>
          )}
        </Toggle>
        <span className="meeting-bar__wide">
          <Toggle
            on={panel === 'transcript'}
            label="Hide transcript"
            offLabel="Show transcript"
            onClick={() =>
              setPanel(panel === 'transcript' ? null : 'transcript')
            }
          >
            <ScrollText />
          </Toggle>
        </span>
      </div>
    </footer>
  )
}

const NOTICE_FOR = 8000

function RoomNotice({ text }: { text: string }) {
  const [hidden, setHidden] = useState(false)
  useEffect(() => {
    const timer = setTimeout(() => setHidden(true), NOTICE_FOR)
    return () => clearTimeout(timer)
  }, [])
  if (hidden || !text) return null
  return (
    <p className="meeting-notice" role="status">
      {text}
    </p>
  )
}

function Knock({
  room,
  engine,
  onOpen,
}: {
  room: RoomState
  engine: Engine
  onOpen: () => void
}) {
  const first = room.waiting[0]
  if (!room.member || !first) return null
  return (
    <div className="meeting-knock" role="status">
      <p>
        <b>{first.name}</b> wants to join
        {room.waiting.length > 1 && ` (and ${room.waiting.length - 1} more)`}
      </p>
      <div>
        {room.waiting.length > 1 && (
          <button
            type="button"
            className="btn btn--sm btn--ghost"
            onClick={onOpen}
          >
            View all
          </button>
        )}
        <button
          type="button"
          className="btn btn--sm btn--ghost"
          onClick={() => engine.send({ kind: 'deny', id: first.id })}
        >
          Deny
        </button>
        <button
          type="button"
          className="btn btn--sm btn--primary"
          onClick={() => engine.send({ kind: 'admit', id: first.id })}
        >
          Admit
        </button>
      </div>
    </div>
  )
}

function useShortcuts(
  engine: Engine | null,
  self: Controls['self'],
  captions: boolean,
  setCaptions: (on: boolean) => void,
) {
  useEffect(() => {
    if (!engine) return
    const keys: Record<string, () => void> = {
      m: () => engine.setMic(!self.mic),
      v: () => void engine.setCamera(!self.camera).catch(() => undefined),
      c: () => setCaptions(!captions),
      h: () => engine.setHand(!self.hand),
    }
    const press = (event: KeyboardEvent) => {
      const target = event.target as HTMLElement
      if (
        event.metaKey ||
        event.ctrlKey ||
        event.altKey ||
        target.closest('input, textarea, select, [contenteditable]')
      )
        return
      keys[event.key.toLowerCase()]?.()
    }
    document.addEventListener('keydown', press)
    return () => document.removeEventListener('keydown', press)
  }, [engine, self, captions, setCaptions])
}

const OUTCOMES: Partial<
  Record<Phase, { mood: Mood; title: string; text: string }>
> = {
  connecting: {
    mood: 'waiting',
    title: 'Joining…',
    text: 'Connecting you to the meeting.',
  },
  waiting: {
    mood: 'waiting',
    title: 'Asking to join…',
    text: 'Someone in the meeting will let you in soon.',
  },
  denied: {
    mood: 'stopped',
    title: 'You were not let in',
    text: 'Someone in the meeting declined your request to join.',
  },
  removed: {
    mood: 'stopped',
    title: 'You were removed from the meeting',
    text: 'The host removed you from this meeting.',
  },
  ended: {
    mood: 'sleeping',
    title: 'The meeting has ended',
    text: 'Thanks for joining.',
  },
  dropped: {
    mood: 'stopped',
    title: 'You lost the connection',
    text: 'Check your network, then rejoin.',
  },
}

function Outcome({
  room,
  onLeave,
  onRejoin,
  ended,
}: {
  room: RoomState
  onLeave: () => void
  onRejoin?: (() => void) | undefined
  ended?: ReactNode
}) {
  const outcome = OUTCOMES[room.phase] ?? OUTCOMES.connecting
  if (!outcome) return null
  const waiting = room.phase === 'connecting' || room.phase === 'waiting'
  return (
    <div className="meeting-outcome night" data-phase={room.phase}>
      <Cube mood={outcome.mood} size={112} />
      <h1>{outcome.title}</h1>
      <p role="status">
        {room.phase === 'dropped' && room.notice ? room.notice : outcome.text}
      </p>
      {room.phase === 'ended' && ended}
      <div className="meeting-outcome__actions">
        {room.phase === 'dropped' && onRejoin && (
          <button type="button" className="btn btn--primary" onClick={onRejoin}>
            Rejoin
          </button>
        )}
        <button type="button" className="btn" onClick={onLeave}>
          {waiting ? 'Cancel' : 'Close'}
        </button>
      </div>
    </div>
  )
}

export function MeetingRoom({
  joining,
  invite,
  meeting,
  ended,
  onLeave,
  onRejoin,
}: {
  joining: Joining
  invite?: ReactNode
  meeting?: Meeting | undefined
  ended?: ReactNode
  onLeave: () => void
  onRejoin?: () => void
}) {
  const { room, self, engine } = useMeeting(joining)
  const [captions, setCaptions] = useState(true)
  const [panel, setPanel] = useState<PanelTab | null>(null)
  const main = useRef<HTMLDivElement>(null)
  useLevels(main, engine, room.you)
  useShortcuts(
    room.phase === 'live' ? engine.current : null,
    self,
    captions,
    setCaptions,
  )
  const current = engine.current
  if (room.phase !== 'live' || !current)
    return (
      <Outcome
        room={room}
        onLeave={onLeave}
        onRejoin={onRejoin}
        ended={ended}
      />
    )
  const presenterSlot = stage(room)
  const presenter = room.participants.find(
    (participant) => participant.slot === presenterSlot,
  )
  const controls = {
    room,
    engine: current,
    self,
    captions,
    panel,
    setCaptions,
    setPanel,
    onLeave,
  }
  return (
    <div className="meeting night" data-panel={panel ?? undefined}>
      <header className="meeting-top">
        <h1>{room.title}</h1>
        <span className="meeting-top__live">
          <span className="meeting-top__dot" aria-hidden="true" />
          Transcribing
        </span>
        {room.locked && (
          <span className="meeting-top__flag">
            <Lock /> Locked
          </span>
        )}
        {room.preparing && (
          <span className="meeting-top__flag" role="status">
            Getting speech ready
            <progress
              max={1}
              value={room.preparing.share}
              aria-label="Downloading speech models"
            />
          </span>
        )}
      </header>
      <main ref={main} className="meeting-main">
        {presenter ? (
          <Stage room={room} engine={current} presenter={presenter} />
        ) : (
          <Grid room={room} engine={current} people={room.participants} />
        )}
        {captions && <CaptionBand room={room} />}
        <Knock room={room} engine={current} onOpen={() => setPanel('people')} />
        <RoomNotice key={room.notice} text={room.notice} />
      </main>
      {panel && (
        <Panel
          tab={panel}
          room={room}
          engine={current}
          invite={invite}
          meeting={meeting}
          onTab={setPanel}
          onClose={() => setPanel(null)}
        />
      )}
      <Bar {...controls} />
    </div>
  )
}
