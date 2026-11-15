import { Link } from '@tanstack/react-router'
import { Box, UsersRound, Video } from 'lucide-react'
import { useState } from 'react'
import { Cube } from '../../components/Cube'
import { Alert, Skeleton } from '../../components/primitives'
import { formatTime, nowIn } from '../calendar/model'
import { FULL_DAY, formatDay } from '../calendar/parts'
import { useMeetings, type MeetingSummary } from './api'
import { useZone } from './MeetingForm'
import { NewMeeting } from './NewMeeting'
import { lengthOf, sectionsOf, type Day } from './schedule'
import './screens.css'

export function Meetings() {
  const [creating, setCreating] = useState(false)
  return (
    <div className="page">
      <header className="page__head meetings-head">
        <h1>Meetings</h1>
        <button
          type="button"
          className="btn btn--primary btn--sm"
          onClick={() => setCreating(true)}
        >
          <Video aria-hidden="true" /> New meeting
        </button>
      </header>
      <div className="page__body">
        <Listing onNew={() => setCreating(true)} />
      </div>
      <NewMeeting open={creating} onClose={() => setCreating(false)} />
    </div>
  )
}

function Listing({ onNew }: { onNew: () => void }) {
  const meetings = useMeetings()
  const zone = useZone()
  if (meetings.isPending)
    return (
      <div className="meetings" aria-busy="true">
        <Skeleton lines={2} width="40%" />
        <Skeleton lines={4} />
      </div>
    )
  if (meetings.isError)
    return (
      <div className="meetings">
        <Alert>Your meetings could not be read. Retry shortly.</Alert>
      </div>
    )
  if (meetings.data.length === 0)
    return (
      <div className="empty meetings__empty">
        <Cube size={56} />
        <h2>No meetings yet</h2>
        <p>
          Talk face to face with people and agents. Start one now or put it on
          the calendar.
        </p>
        <button type="button" className="btn btn--primary" onClick={onNew}>
          <Video aria-hidden="true" /> New meeting
        </button>
      </div>
    )
  const { live, upcoming, past } = sectionsOf(meetings.data, zone)
  return (
    <div className="meetings">
      {live.length > 0 && (
        <section aria-labelledby="meetings-live">
          <h2 id="meetings-live" className="meetings__head">
            Live now
          </h2>
          <ul className="meetings__live">
            {live.map((meeting) => (
              <LiveRow key={meeting.id} meeting={meeting} zone={zone} />
            ))}
          </ul>
        </section>
      )}
      {upcoming.length > 0 && (
        <section aria-labelledby="meetings-upcoming">
          <h2 id="meetings-upcoming" className="meetings__head">
            Upcoming
          </h2>
          {upcoming.map((day) => (
            <DayGroup key={day.day} group={day} zone={zone} />
          ))}
        </section>
      )}
      {past.length > 0 && (
        <section aria-labelledby="meetings-past">
          <h2 id="meetings-past" className="meetings__head">
            Past
          </h2>
          <ul className="meetings__list">
            {past.map((meeting) => (
              <PastRow key={meeting.id} meeting={meeting} zone={zone} />
            ))}
          </ul>
        </section>
      )}
    </div>
  )
}

function Counts({ meeting }: { meeting: MeetingSummary }) {
  return (
    <span className="meeting-row__counts">
      {meeting.people > 0 && (
        <span title="People invited">
          <UsersRound aria-hidden="true" />
          {meeting.people}
          <span className="sr-only"> people</span>
        </span>
      )}
      {meeting.agents > 0 && (
        <span title="Agents invited">
          <Box aria-hidden="true" />
          {meeting.agents}
          <span className="sr-only"> agents</span>
        </span>
      )}
    </span>
  )
}

function TitleLink({ meeting }: { meeting: MeetingSummary }) {
  return (
    <Link
      to="/meetings/$meetingId"
      params={{ meetingId: meeting.id }}
      className="meeting-row__title"
    >
      {meeting.title}
    </Link>
  )
}

function LiveRow({ meeting, zone }: { meeting: MeetingSummary; zone: string }) {
  const since = meeting.started_at ?? meeting.starts_at
  return (
    <li className="meeting-row meeting-row--live">
      <span className="meeting-row__well" aria-hidden="true">
        <Video />
      </span>
      <span className="meeting-row__main">
        <TitleLink meeting={meeting} />
        <span className="meeting-row__detail">
          <span className="live-dot meeting-row__dot" aria-hidden="true" />
          {since ? `Started ${formatTime(since, zone)}` : 'Live'} · Hosted by{' '}
          {meeting.owner_name}
        </span>
      </span>
      <Counts meeting={meeting} />
      <Link
        to="/meetings/$meetingId"
        params={{ meetingId: meeting.id }}
        search={{ join: true }}
        className="btn btn--primary btn--sm meeting-row__join"
      >
        Join
      </Link>
    </li>
  )
}

function dayName(day: string, zone: string) {
  if (!day) return 'Not scheduled'
  const today = nowIn(zone).day
  const label = formatDay(day, FULL_DAY)
  if (day === today) return `Today · ${label}`
  return label
}

function DayGroup({ group, zone }: { group: Day; zone: string }) {
  return (
    <div className="meetings__day">
      <h3 className="meetings__date">{dayName(group.day, zone)}</h3>
      <ul className="meetings__list">
        {group.meetings.map((meeting) => (
          <li key={meeting.id} className="meeting-row">
            <span className="meeting-row__time tabular">
              {meeting.starts_at && meeting.ends_at ? (
                <>
                  <span>{formatTime(meeting.starts_at, zone)}</span>
                  <span className="faint">
                    {formatTime(meeting.ends_at, zone)}
                  </span>
                </>
              ) : (
                <span className="faint">Instant</span>
              )}
            </span>
            <span className="meeting-row__main">
              <TitleLink meeting={meeting} />
              <span className="meeting-row__detail">
                Hosted by {meeting.owner_name}
              </span>
            </span>
            <Counts meeting={meeting} />
          </li>
        ))}
      </ul>
    </div>
  )
}

function PastRow({ meeting, zone }: { meeting: MeetingSummary; zone: string }) {
  const start = meeting.started_at ?? meeting.starts_at
  const end = meeting.ended_at
  const date = start
    ? formatDay(nowIn(zone, Date.parse(start)).day, {
        day: 'numeric',
        month: 'short',
      })
    : ''
  return (
    <li className="meeting-row meeting-row--past">
      <span className="meeting-row__time tabular">
        <span>{date}</span>
        {start && <span className="faint">{formatTime(start, zone)}</span>}
      </span>
      <span className="meeting-row__main">
        <TitleLink meeting={meeting} />
        <span className="meeting-row__detail">
          {start && end ? `${lengthOf(start, end)} · ` : ''}Hosted by{' '}
          {meeting.owner_name}
        </span>
      </span>
      <Counts meeting={meeting} />
    </li>
  )
}
