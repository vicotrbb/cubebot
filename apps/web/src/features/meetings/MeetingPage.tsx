import { Link, useNavigate } from '@tanstack/react-router'
import {
  ArrowLeft,
  Clock,
  Ellipsis,
  Link2,
  Link2Off,
  Lock,
  Pencil,
  RefreshCw,
  Trash2,
  UserRound,
  Video,
} from 'lucide-react'
import { useState } from 'react'
import { api, failure, ok } from '../../api/client'
import type { Schemas } from '../../api/types'
import { Cube } from '../../components/Cube'
import {
  Alert,
  confirmAction,
  CopyButton,
  Dialog,
  Field,
  Menu,
  Skeleton,
  Switch,
} from '../../components/primitives'
import { formatTime } from '../calendar/model'
import { FULL_DAY, formatDay } from '../calendar/parts'
import { useMeetingRecord, useRefresh, type Meeting } from './api'
import { GreenRoom } from './GreenRoom'
import { InvitePanel, OpenJoin } from './InvitePanel'
import { ModeField, useZone, WhenFields } from './MeetingForm'
import { liveUrl } from './PublicMeeting'
import { Invited, StateChip, Summary, Transcript } from './Record'
import { MeetingRoom } from './Room'
import { instantsOf, lengthOf, whenOf } from './schedule'
import type { Joining } from './useMeeting'
import './meetings.css'
import './screens.css'

type Stage = 'detail' | 'green' | Joining

export function MeetingPage({ id, join }: { id: string; join: boolean }) {
  const record = useMeetingRecord(id)
  const navigate = useNavigate()
  const [stage, setStage] = useState<Stage>(join ? 'green' : 'detail')
  const zone = useZone()
  const toDetail = () => {
    setStage('detail')
    void record.refetch()
    if (join)
      void navigate({
        to: '/meetings/$meetingId',
        params: { meetingId: id },
        search: {},
        replace: true,
      })
  }
  if (record.isPending) return <Loading />
  if (record.isError) return <Missing />
  const meeting = record.data
  if (stage === 'green' && meeting.state !== 'ended')
    return (
      <GreenRoom
        title={meeting.title}
        detail={hostLine(meeting, zone)}
        guest={false}
        url={liveUrl(`/api/meetings/${meeting.id}/live`)}
        onBack={toDetail}
        onJoin={setStage}
      />
    )
  if (typeof stage === 'object')
    return (
      <MeetingRoom
        joining={stage}
        invite={<InvitePanel meeting={meeting} />}
        meeting={meeting}
        ended={
          <button type="button" className="link-btn" onClick={toDetail}>
            Open the meeting
          </button>
        }
        onLeave={toDetail}
        onRejoin={() => {
          setStage('green')
          void record.refetch()
        }}
      />
    )
  return (
    <Detail meeting={meeting} zone={zone} onJoin={() => setStage('green')} />
  )
}

function hostLine(meeting: Meeting, zone: string) {
  const host = meeting.owner ? 'You host' : `Hosted by ${meeting.owner_name}`
  if (!meeting.starts_at || !meeting.ends_at) return host
  return `${formatTime(meeting.starts_at, zone)} to ${formatTime(meeting.ends_at, zone)} · ${host}`
}

function Loading() {
  return (
    <div className="page">
      <header className="page__head">
        <span className="skeleton" style={{ width: 220 }} />
      </header>
      <div className="page__body">
        <div className="meeting-detail">
          <Skeleton lines={6} />
        </div>
      </div>
    </div>
  )
}

function Missing() {
  return (
    <div className="page">
      <div className="page__body">
        <div className="empty">
          <Cube mood="stopped" size={56} />
          <h2>This meeting is not here</h2>
          <p>It was cancelled, or you are not invited to it.</p>
          <Link to="/meetings" className="btn">
            All meetings
          </Link>
        </div>
      </div>
    </div>
  )
}

function whenWords(meeting: Meeting, zone: string) {
  const start = meeting.starts_at ?? meeting.started_at
  if (!start)
    return { day: 'Instant meeting', time: 'Starts when someone joins' }
  const end = meeting.ends_at ?? meeting.ended_at
  const day = formatDay(whenOf(start, end ?? start, zone).day, FULL_DAY)
  const time = end
    ? `${formatTime(start, zone)} to ${formatTime(end, zone)} · ${lengthOf(start, end)}`
    : `Started ${formatTime(start, zone)}`
  return { day: meeting.starts_at ? day : `Instant · ${day}`, time }
}

function Detail({
  meeting,
  zone,
  onJoin,
}: {
  meeting: Meeting
  zone: string
  onJoin: () => void
}) {
  const [editing, setEditing] = useState(false)
  const change = useChange(meeting)
  const when = whenWords(meeting, zone)
  return (
    <div className="page">
      <header className="page__head meeting-head">
        <Link
          to="/meetings"
          className="btn btn--ghost btn--icon btn--sm"
          aria-label="All meetings"
          title="All meetings"
        >
          <ArrowLeft />
        </Link>
        <h1>{meeting.title}</h1>
        <StateChip state={meeting.state} />
        <div className="meeting-head__actions">
          {meeting.link && (
            <CopyButton text={meeting.link} label="Copy meeting link" />
          )}
          {meeting.owner && (
            <OwnerMenu
              meeting={meeting}
              run={change.run}
              onEdit={() => setEditing(true)}
            />
          )}
          {meeting.state !== 'ended' && (
            <button
              type="button"
              className="btn btn--primary btn--sm"
              onClick={onJoin}
            >
              <Video aria-hidden="true" /> Join
            </button>
          )}
        </div>
      </header>
      <div className="page__body">
        <div className="meeting-detail" data-state={meeting.state}>
          <div className="meeting-detail__facts">
            <Alert>{change.error}</Alert>
            <dl className="meeting-facts">
              <div>
                <dt>
                  <Clock aria-label="When" role="img" />
                </dt>
                <dd>
                  <span>{when.day}</span>
                  <span className="faint tabular">{when.time}</span>
                </dd>
              </div>
              <div>
                <dt>
                  <UserRound aria-label="Host" role="img" />
                </dt>
                <dd>
                  <span>{meeting.owner ? 'You host' : meeting.owner_name}</span>
                  {meeting.locked && (
                    <span className="faint meeting-facts__lock">
                      <Lock aria-hidden="true" /> Locked: nobody else can join
                    </span>
                  )}
                </dd>
              </div>
            </dl>
            {meeting.description && (
              <p className="meeting-detail__about">{meeting.description}</p>
            )}
          </div>
          <aside className="meeting-detail__side">
            <section aria-labelledby="meeting-invited">
              <h2 id="meeting-invited">
                Invited{' '}
                <span className="faint tabular">
                  {meeting.invitations.length}
                </span>
              </h2>
              <Invited meeting={meeting} zone={zone} />
            </section>
            {meeting.owner && meeting.state !== 'ended' && (
              <section aria-labelledby="meeting-access">
                <h2 id="meeting-access">Access</h2>
                <OpenJoin meeting={meeting} />
                <LockSwitch meeting={meeting} />
              </section>
            )}
          </aside>
          <div className="meeting-detail__record">
            {meeting.state === 'ended' && (
              <section aria-labelledby="meeting-summary">
                <h2 id="meeting-summary">Summary</h2>
                <Summary meeting={meeting} />
              </section>
            )}
            <section aria-labelledby="meeting-transcript">
              <h2 id="meeting-transcript">Transcript</h2>
              <Transcript meeting={meeting} zone={zone} />
            </section>
          </div>
        </div>
      </div>
      {editing && (
        <EditMeeting
          meeting={meeting}
          zone={zone}
          onClose={() => setEditing(false)}
        />
      )}
    </div>
  )
}

function useChange(meeting: Meeting) {
  const refresh = useRefresh()
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  const run = async (write: () => Promise<unknown>) => {
    setBusy(true)
    setError('')
    try {
      await write()
      await refresh()
      return true
    } catch (caught) {
      setError(failure(caught))
      return false
    } finally {
      setBusy(false)
    }
  }
  const patch = (body: () => Schemas['ChangeMeeting']) =>
    run(() =>
      ok(
        api.PATCH('/api/meetings/{id}', {
          params: { path: { id: meeting.id } },
          body: body(),
        }),
      ),
    )
  return { busy, error, run, patch }
}

function LockSwitch({ meeting }: { meeting: Meeting }) {
  const { busy, error, patch } = useChange(meeting)
  return (
    <>
      <div className="capability">
        <div>
          <p className="capability__name">Lock the meeting</p>
          <p className="faint small">
            Nobody new joins, members included, until you unlock it.
          </p>
        </div>
        <Switch
          checked={meeting.locked}
          disabled={busy}
          label="Lock the meeting"
          onChange={(locked) => void patch(() => ({ locked }))}
        />
      </div>
      <Alert>{error}</Alert>
    </>
  )
}

function OwnerMenu({
  meeting,
  run,
  onEdit,
}: {
  meeting: Meeting
  run: ReturnType<typeof useChange>['run']
  onEdit: () => void
}) {
  const navigate = useNavigate()
  const link = (enabled: boolean) =>
    run(() =>
      ok(
        api.POST('/api/meetings/{id}/link', {
          params: { path: { id: meeting.id } },
          body: { enabled },
        }),
      ),
    )
  const remove = async () => {
    const yes = await confirmAction({
      title: `Delete ${meeting.title}?`,
      body: 'Its invitations, transcript and summary are deleted too. Invited people are not told.',
      action: 'Delete meeting',
    })
    if (
      yes &&
      (await run(() =>
        ok(
          api.DELETE('/api/meetings/{id}', {
            params: { path: { id: meeting.id } },
          }),
        ),
      ))
    )
      await navigate({ to: '/meetings' })
  }
  return (
    <Menu
      className="menu--right"
      trigger={<Ellipsis />}
      triggerClassName="btn btn--ghost btn--icon btn--sm"
      label="Meeting actions"
    >
      {meeting.state !== 'ended' && (
        <button type="button" className="menu__item" onClick={onEdit}>
          <Pencil /> Edit
        </button>
      )}
      {meeting.link ? (
        <>
          <button
            type="button"
            className="menu__item"
            onClick={() => void link(true)}
          >
            <RefreshCw /> New link
          </button>
          <button
            type="button"
            className="menu__item"
            onClick={() => void link(false)}
          >
            <Link2Off /> Turn link off
          </button>
        </>
      ) : (
        <button
          type="button"
          className="menu__item"
          onClick={() => void link(true)}
        >
          <Link2 /> Turn link on
        </button>
      )}
      {meeting.state !== 'live' && (
        <>
          <div className="menu__sep" role="separator" />
          <button
            type="button"
            className="menu__item menu__item--danger"
            onClick={() => void remove()}
          >
            <Trash2 /> Delete
          </button>
        </>
      )}
    </Menu>
  )
}

function EditMeeting({
  meeting,
  zone,
  onClose,
}: {
  meeting: Meeting
  zone: string
  onClose: () => void
}) {
  const { busy, error, patch } = useChange(meeting)
  const scheduled = Boolean(meeting.starts_at && meeting.ends_at)
  const [title, setTitle] = useState(meeting.title)
  const [description, setDescription] = useState(meeting.description)
  const [mode, setMode] = useState(meeting.agent_mode)
  const [times, setTimes] = useState(() =>
    meeting.starts_at && meeting.ends_at
      ? whenOf(meeting.starts_at, meeting.ends_at, zone)
      : { day: '', start: '', end: '' },
  )
  const save = async () => {
    const saved = await patch(() => ({
      title: title.trim(),
      description: description.trim(),
      agent_mode: mode,
      ...(scheduled ? instantsOf(times, zone) : {}),
    }))
    if (saved) onClose()
  }
  return (
    <Dialog open title="Edit meeting" onClose={onClose}>
      <form
        className="form meeting-form"
        onSubmit={(event) => {
          event.preventDefault()
          void save()
        }}
      >
        <Field label="Title">
          <input
            value={title}
            required
            maxLength={200}
            onChange={(event) => setTitle(event.target.value)}
          />
        </Field>
        {scheduled && (
          <WhenFields when={times} zone={zone} onChange={setTimes} />
        )}
        <Field label="Description" hint="Optional">
          <textarea
            rows={3}
            maxLength={4000}
            value={description}
            onChange={(event) => setDescription(event.target.value)}
          />
        </Field>
        <ModeField value={mode} onChange={setMode} />
        <Alert>{error}</Alert>
        <div className="actions dialog__actions">
          <button type="button" className="btn btn--ghost" onClick={onClose}>
            Cancel
          </button>
          <button className="btn btn--primary" disabled={busy || !title.trim()}>
            {busy ? 'Saving…' : 'Save'}
          </button>
        </div>
      </form>
    </Dialog>
  )
}
