import { useQuery } from '@tanstack/react-query'
import { useNavigate } from '@tanstack/react-router'
import { useState } from 'react'
import { api, failure, ok } from '../../api/client'
import { useSession } from '../../app/queries'
import { Cube } from '../../components/Cube'
import { Alert } from '../../components/primitives'
import { GreenRoom } from './GreenRoom'
import { MeetingRoom } from './Room'
import type { Joining } from './useMeeting'
import './meetings.css'

type Lookup = {
  title: string
  host: string
  state: 'scheduled' | 'live' | 'ended'
  locked: boolean
  lobby: boolean
}

async function lookup(token: string): Promise<Lookup | null> {
  const answer = await fetch(
    `/api/public/meetings/${encodeURIComponent(token)}`,
    {
      credentials: 'omit',
    },
  )
  if (answer.status === 404) return null
  if (!answer.ok)
    throw new Error(answer.status === 429 ? 'busy' : 'unavailable')
  return (await answer.json()) as Lookup
}

export const liveUrl = (path: string) =>
  `${location.protocol === 'https:' ? 'wss' : 'ws'}://${location.host}${path}`

function Notice({
  mood,
  title,
  text,
}: {
  mood: 'sleeping' | 'stopped' | 'waiting'
  title: string
  text: string
}) {
  return (
    <div className="meeting-outcome night">
      <Cube mood={mood} size={112} />
      <h1>{title}</h1>
      <p>{text}</p>
    </div>
  )
}

function detail(meeting: Lookup) {
  const who = `Hosted by ${meeting.host}`
  if (meeting.state === 'live') return `${who} · happening now`
  return meeting.lobby ? `${who} · someone will let you in` : who
}

function AsMember({ token, name }: { token: string; name: string }) {
  const navigate = useNavigate()
  const [busy, setBusy] = useState(false)
  const [problem, setProblem] = useState('')
  const join = async () => {
    setBusy(true)
    try {
      const joined = await ok(
        api.POST('/api/meetings/link/{token}/join', {
          params: { path: { token } },
        }),
      )
      await navigate({
        to: '/meetings/$meetingId',
        params: { meetingId: joined.id },
        search: { join: true },
      })
    } catch (caught) {
      setProblem(failure(caught))
      setBusy(false)
    }
  }
  return (
    <div className="green-room__member">
      <p>You are signed in as {name}.</p>
      <button
        type="button"
        className="btn btn--primary"
        disabled={busy}
        onClick={() => void join()}
      >
        Join as {name}
      </button>
      <Alert>{problem}</Alert>
    </div>
  )
}

export function PublicMeeting({ token }: { token: string }) {
  const meeting = useQuery({
    queryKey: ['public-meeting', token],
    queryFn: () => lookup(token),
    retry: false,
  })
  const session = useSession()
  const [joining, setJoining] = useState<Joining | null>(null)
  if (meeting.isPending)
    return <div className="meeting-outcome night" aria-busy="true" />
  if (meeting.isError)
    return (
      <Notice
        mood="stopped"
        title="This meeting is not answering"
        text={
          meeting.error.message === 'busy'
            ? 'Too many tries from this network. Wait a minute and open the link again.'
            : 'Cubebot could not be reached. Try again in a moment.'
        }
      />
    )
  const found = meeting.data
  if (!found)
    return (
      <Notice
        mood="stopped"
        title="This link does not work"
        text="The meeting does not exist or its link was turned off. Ask the host for a new one."
      />
    )
  if (found.state === 'ended')
    return (
      <Notice
        mood="sleeping"
        title="This meeting has ended"
        text={`${found.title}, hosted by ${found.host}, is over.`}
      />
    )
  if (found.locked)
    return (
      <Notice
        mood="stopped"
        title="This meeting is locked"
        text="The host is not letting anyone else in right now."
      />
    )
  if (joining)
    return (
      <MeetingRoom
        joining={joining}
        onLeave={() => setJoining(null)}
        onRejoin={() => setJoining(null)}
      />
    )
  const member = session.data
  return (
    <GreenRoom
      title={found.title}
      detail={detail(found)}
      guest
      url={liveUrl(`/api/public/meetings/${encodeURIComponent(token)}/live`)}
      joinLabel={found.lobby ? 'Ask to join' : 'Join now'}
      aside={
        member ? (
          <AsMember token={token} name={member.name || member.login} />
        ) : null
      }
      onJoin={setJoining}
    />
  )
}
