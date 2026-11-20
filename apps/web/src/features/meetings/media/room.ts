import type { Avatar } from '../../../components/Cube'
import type { Stream } from './wire'

export type ParticipantKind = 'person' | 'guest' | 'agent'
export type AgentState = 'listening' | 'thinking' | 'speaking'

export type Participant = {
  slot: number
  kind: ParticipantKind
  name: string
  account?: string
  agent?: string
  avatar?: Avatar
  host: boolean
  mic: boolean
  camera: boolean
  screen: boolean
  hand: boolean
  listening?: boolean
  can_present?: boolean
}

export type Track = { codec: string | null; width: number; height: number }

export type Caption = {
  id: string
  source: number
  text: string
  final: boolean
  at: number
}

export type Phase =
  | 'connecting'
  | 'waiting'
  | 'live'
  | 'reconnecting'
  | 'denied'
  | 'removed'
  | 'ended'
  | 'dropped'

export type Server =
  | { kind: 'waiting' }
  | {
      kind: 'welcome'
      you: number
      title: string
      host: boolean
      member: boolean
      open_join: boolean
      locked: boolean
      rejoin?: string
    }
  | { kind: 'roster'; participants: Participant[] }
  | {
      kind: 'track'
      source: number
      stream: Stream
      codec: string | null
      width: number
      height: number
    }
  | { kind: 'keyframe'; stream: Stream }
  | {
      kind: 'caption'
      source: number
      id: string
      text: string
      final: boolean
    }
  | { kind: 'agent'; source: number; state: AgentState }
  | { kind: 'interrupt'; source: number }
  | {
      kind: 'desktop'
      source: number
      tiles?: { x: number; y: number; png: string }[]
      ended?: boolean
    }
  | { kind: 'lobby'; waiting: { id: string; name: string }[] }
  | { kind: 'meeting'; locked: boolean; open_join: boolean }
  | { kind: 'preparing'; model: string; received: number; total: number }
  | { kind: 'notice'; text: string }
  | { kind: 'denied' }
  | { kind: 'removed' }
  | { kind: 'ended' }

export type Room = {
  phase: Phase
  you: number | null
  title: string
  host: boolean
  member: boolean
  openJoin: boolean
  locked: boolean
  participants: Participant[]
  tracks: Record<string, Track>
  captions: Caption[]
  agents: Record<number, AgentState>
  presenting: number[]
  waiting: { id: string; name: string }[]
  preparing: { model: string; share: number } | null
  notice: string
  retries: number
}

export const EMPTY_ROOM: Room = {
  phase: 'connecting',
  you: null,
  title: '',
  host: false,
  member: false,
  openJoin: false,
  locked: false,
  participants: [],
  tracks: {},
  captions: [],
  agents: {},
  presenting: [],
  waiting: [],
  preparing: null,
  notice: '',
  retries: 0,
}

export const RETRY_DELAYS = [1000, 2000, 4000, 8000, 15000, 15000, 15000]

const CAPTIONS_KEPT = 200

export const trackKey = (source: number, stream: Stream) =>
  `${source}:${stream}`

function captioned(captions: Caption[], line: Omit<Caption, 'at'>) {
  const index = captions.findIndex((caption) => caption.id === line.id)
  const next = { ...line, at: Date.now() }
  if (index === -1) return [...captions, next].slice(-CAPTIONS_KEPT)
  return captions.map((caption, at) => (at === index ? next : caption))
}

function rostered(room: Room, participants: Participant[]): Room {
  const slots = new Set(participants.map((participant) => participant.slot))
  const screens = participants
    .filter((participant) => participant.screen)
    .map((participant) => participant.slot)
  const kept = room.presenting.filter(
    (slot) => slots.has(slot) && screens.includes(slot),
  )
  const added = screens.filter((slot) => !kept.includes(slot))
  return { ...room, participants, presenting: [...kept, ...added] }
}

function ending(room: Room, message: Server): Room | null {
  switch (message.kind) {
    case 'waiting':
      return { ...room, phase: 'waiting' }
    case 'denied':
    case 'removed':
    case 'ended':
      return { ...room, phase: message.kind }
    case 'welcome':
      return {
        ...room,
        phase: 'live',
        retries: 0,
        tracks: {},
        you: message.you,
        title: message.title,
        host: message.host,
        member: message.member,
        openJoin: message.open_join,
        locked: message.locked,
      }
    default:
      return null
  }
}

export function reduce(room: Room, message: Server): Room {
  const changed = ending(room, message)
  if (changed) return changed
  switch (message.kind) {
    case 'roster':
      return rostered(room, message.participants)
    case 'track':
      return {
        ...room,
        tracks: {
          ...room.tracks,
          [trackKey(message.source, message.stream)]: {
            codec: message.codec,
            width: message.width,
            height: message.height,
          },
        },
      }
    case 'caption':
      return { ...room, captions: captioned(room.captions, message) }
    case 'agent':
      return {
        ...room,
        agents: { ...room.agents, [message.source]: message.state },
      }
    case 'lobby':
      return { ...room, waiting: message.waiting }
    case 'meeting':
      return { ...room, locked: message.locked, openJoin: message.open_join }
    case 'preparing':
      return {
        ...room,
        preparing: message.total
          ? { model: message.model, share: message.received / message.total }
          : null,
      }
    case 'notice':
      return { ...room, notice: message.text }
    default:
      return room
  }
}

export function dropped(room: Room): Room {
  return room.phase === 'connecting' ||
    room.phase === 'live' ||
    room.phase === 'reconnecting' ||
    room.phase === 'waiting'
    ? { ...room, phase: 'dropped' }
    : room
}

export function lost(room: Room, reason: string): Room {
  const inside = room.phase === 'live' || room.phase === 'reconnecting'
  if (!reason && inside && room.retries < RETRY_DELAYS.length)
    return {
      ...room,
      phase: 'reconnecting',
      retries: room.retries + 1,
      notice: '',
    }
  return { ...dropped(room), notice: reason || room.notice }
}

export function stage(room: Room) {
  return room.presenting.at(-1) ?? null
}

export function speakerName(room: Room, source: number) {
  const found = room.participants.find(
    (participant) => participant.slot === source,
  )
  if (!found) return 'Someone'
  if (found.slot === room.you) return 'You'
  return found.kind === 'guest' ? `${found.name} (guest)` : found.name
}
