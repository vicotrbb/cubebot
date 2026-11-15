import { ChevronRight, Users } from 'lucide-react'
import { useState, type ReactNode } from 'react'
import type { Turn } from '../../api/client'
import { clock } from '../call/call'
import { Markdown } from '../chat/Markdown'
import { isUnsettled } from '../chat/useConversation'
import './meeting-message.css'

type Record = NonNullable<Turn['meeting']>

const DAY = new Intl.DateTimeFormat(undefined, {
  dateStyle: 'medium',
  timeStyle: 'short',
})
const TIME = new Intl.DateTimeFormat(undefined, { timeStyle: 'short' })

function named(someone: { kind: string; name: string }) {
  return someone.kind === 'guest' ? `${someone.name} (guest)` : someone.name
}

function overview(meeting: Record, live: boolean) {
  const count = meeting.present.length
  return [
    live ? 'Live now' : clock(meeting.duration_seconds ?? 0),
    `${count} ${count === 1 ? 'participant' : 'participants'}`,
  ].join(' · ')
}

export function MeetingMessage({
  meeting,
  children,
}: {
  meeting: Record
  children: ReactNode
}) {
  const live = meeting.ended_at === null || meeting.ended_at === undefined
  const unsettled = meeting.turns.some(isUnsettled)
  const [opened, setOpened] = useState<boolean | null>(null)
  const open = opened ?? (live || unsettled)
  const started = meeting.started_at ?? null
  return (
    <article className="meeting-message" data-open={open} data-live={live}>
      <button
        type="button"
        className="meeting-message__row"
        aria-expanded={open}
        onClick={() => setOpened(!open)}
      >
        <span className="meeting-message__icon" aria-hidden="true">
          <Users />
        </span>
        <span className="meeting-message__text">
          <span className="meeting-message__title">
            <span className="meeting-message__name">
              Meeting · {meeting.title}
            </span>
            {started !== null && (
              <time
                className="meeting-message__time"
                dateTime={new Date(started * 1000).toISOString()}
              >
                {DAY.format(started * 1000)}
              </time>
            )}
          </span>
          <span className="meeting-message__summary">
            {overview(meeting, live)}
            {unsettled && ' · still working'}
          </span>
        </span>
        <ChevronRight className="meeting-message__chevron" aria-hidden="true" />
      </button>
      {open && <Details meeting={meeting}>{children}</Details>}
    </article>
  )
}

function Details({
  meeting,
  children,
}: {
  meeting: Record
  children: ReactNode
}) {
  return (
    <div className="meeting-message__body">
      {meeting.present.length > 0 && (
        <p className="meeting-message__present">
          <span className="meeting-message__label">With</span>{' '}
          {meeting.present.map(named).join(', ')}
        </p>
      )}
      {meeting.summary && (
        <section className="meeting-message__section" aria-label="Summary">
          <h3 className="meeting-message__label">Summary</h3>
          <Markdown text={meeting.summary} />
        </section>
      )}
      <Transcript meeting={meeting} />
      {meeting.turns.length > 0 && (
        <section
          className="meeting-message__section"
          aria-label="Requests answered"
        >
          <h3 className="meeting-message__label">Requests answered</h3>
          <div className="meeting-message__turns">{children}</div>
        </section>
      )}
      <a className="meeting-message__open" href={`/meetings/${meeting.id}`}>
        Open meeting
      </a>
    </div>
  )
}

function Transcript({ meeting }: { meeting: Record }) {
  if (meeting.transcript.length === 0) {
    return (
      <p className="meeting-message__empty">
        Nothing has been said in this meeting yet.
      </p>
    )
  }
  return (
    <section className="meeting-message__section" aria-label="Transcript">
      <h3 className="meeting-message__label">Transcript</h3>
      <ol className="meeting-message__lines">
        {meeting.transcript.map((line, index) => (
          <li
            key={index}
            className="meeting-line"
            data-own={line.own}
            data-kind={line.kind}
          >
            <span className="meeting-line__who">
              {named({ kind: line.kind, name: line.speaker })}
            </span>
            <time
              className="meeting-line__at"
              dateTime={new Date(line.at * 1000).toISOString()}
            >
              {TIME.format(line.at * 1000)}
            </time>
            <p className="meeting-line__text">{line.text}</p>
          </li>
        ))}
      </ol>
      {meeting.lines > meeting.transcript.length && (
        <p className="meeting-message__more">
          Showing the first {meeting.transcript.length} of {meeting.lines}{' '}
          lines. Open the meeting for the rest.
        </p>
      )}
    </section>
  )
}
