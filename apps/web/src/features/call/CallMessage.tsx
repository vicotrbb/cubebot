import { ChevronRight, Phone } from 'lucide-react'
import { useState, type ReactNode } from 'react'
import type { Turn } from '../../api/client'
import { isUnsettled } from '../chat/useConversation'
import { clock, providers } from './call'
import './call.css'

type Record = NonNullable<Turn['call']>

const TIME = new Intl.DateTimeFormat(undefined, {
  dateStyle: 'medium',
  timeStyle: 'short',
})

function plural(count: number) {
  return `${count} ${count === 1 ? 'turn' : 'turns'}`
}

function summary(call: Record) {
  const live =
    call.duration_seconds === null || call.duration_seconds === undefined
  return [
    live ? 'On the call now' : clock(call.duration_seconds ?? 0),
    providers(call.stt_provider, call.tts_provider),
    plural(call.turns.length),
  ].join(' · ')
}

export function CallMessage({
  call,
  children,
}: {
  call: Record
  children: ReactNode
}) {
  const unsettled = call.turns.some(isUnsettled)
  const [opened, setOpened] = useState<boolean | null>(null)
  const open = opened ?? unsettled
  return (
    <article className="call-message" data-open={open}>
      <button
        type="button"
        className="call-message__row"
        aria-expanded={open}
        onClick={() => setOpened(!open)}
      >
        <span className="call-message__icon" aria-hidden="true">
          <Phone />
        </span>
        <span className="call-message__text">
          <span className="call-message__title">
            Call
            <time
              className="call-message__time"
              dateTime={new Date(call.started_at * 1000).toISOString()}
            >
              {TIME.format(call.started_at * 1000)}
            </time>
          </span>
          <span className="call-message__summary">
            {summary(call)}
            {unsettled && ' · still working'}
          </span>
        </span>
        <ChevronRight className="call-message__chevron" aria-hidden="true" />
      </button>
      {open &&
        (call.turns.length > 0 ? (
          <div className="call-message__transcript">{children}</div>
        ) : (
          <p className="call-message__empty">Nothing was said on this call.</p>
        ))}
    </article>
  )
}
