import { useEffect, useReducer, useRef, useState } from 'react'
import {
  connect,
  stopTracks,
  type Devices,
  type Engine,
  type Self,
} from './media/engine'
import type { Noise } from './media/noise'
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
  noise: Noise
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
  const mounts = useRef(0)
  useEffect(() => {
    mounts.current += 1
    const mount = mounts.current
    const joined = setup.current
    const started = connect({
      ...joined,
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
      setTimeout(() => {
        if (mounts.current !== mount) return
        stopTracks(joined.local)
        void joined.context.close()
      }, 0)
    }
  }, [])
  return { room, self, engine }
}
