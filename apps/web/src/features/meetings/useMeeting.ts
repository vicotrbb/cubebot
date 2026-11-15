import { useEffect, useReducer, useRef, useState } from 'react'
import { connect, type Devices, type Engine, type Self } from './media/engine'
import {
  dropped,
  EMPTY_ROOM,
  reduce,
  type Room,
  type Server,
} from './media/room'

export type Joining = {
  url: string
  name?: string | undefined
  context: AudioContext
  local: MediaStream
  devices: Devices
  self: Self
}

type Action = { server: Server } | { lost: string }

function roomOf(room: Room, action: Action): Room {
  if ('server' in action) return reduce(room, action.server)
  return { ...dropped(room), notice: action.lost || room.notice }
}

function keepAwake() {
  const lock = navigator.wakeLock?.request('screen').catch(() => null)
  return () => void lock?.then((held) => held?.release())
}

export function useMeeting(joining: Joining) {
  const [room, dispatch] = useReducer(roomOf, EMPTY_ROOM)
  const [self, setSelf] = useState<Self>(joining.self)
  const engine = useRef<Engine | null>(null)
  const setup = useRef(joining)
  useEffect(() => {
    const started = connect({
      ...setup.current,
      onServer: (server) => dispatch({ server }),
      onSelf: setSelf,
      onClosed: (lost) => dispatch({ lost }),
    })
    engine.current = started
    const release = keepAwake()
    return () => {
      release()
      started.close()
      engine.current = null
    }
  }, [])
  return { room, self, engine }
}
