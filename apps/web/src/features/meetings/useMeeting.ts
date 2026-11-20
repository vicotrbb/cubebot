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
  EMPTY_ROOM,
  lost,
  reduce,
  RETRY_DELAYS,
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
  return lost(room, action.lost)
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
  const latest = useRef(joining.self)
  const rejoin = useRef<string | undefined>(undefined)
  const [connection, setConnection] = useState(0)
  useEffect(() => {
    mounts.current += 1
    const mount = mounts.current
    const joined = setup.current
    const release = keepAwake()
    return () => {
      release()
      setTimeout(() => {
        if (mounts.current !== mount) return
        stopTracks(joined.local)
        void joined.context.close()
      }, 0)
    }
  }, [])
  useEffect(() => {
    if (latest.current.screen) {
      latest.current = { ...latest.current, screen: false }
      setSelf(latest.current)
    }
    const started = connect({
      ...setup.current,
      self: latest.current,
      rejoin: rejoin.current,
      onServer: (server) => {
        if (server.kind === 'welcome') rejoin.current = server.rejoin
        dispatch({ server })
      },
      onSelf: (next) => {
        latest.current = next
        setSelf(next)
      },
      onClosed: (reason) => dispatch({ lost: reason }),
    })
    engine.current = started
    return () => {
      started.close()
      if (engine.current === started) engine.current = null
    }
  }, [connection])
  useEffect(() => {
    if (room.phase !== 'reconnecting') return
    const timer = setTimeout(
      () => setConnection((count) => count + 1),
      RETRY_DELAYS[room.retries - 1],
    )
    return () => clearTimeout(timer)
  }, [room.phase, room.retries])
  return { room, self, engine }
}
