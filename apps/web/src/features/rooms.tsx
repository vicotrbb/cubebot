import { useEffect, useState } from 'react'
import {
  api,
  failure,
  messageProblem,
  ok,
  rejectedInput,
  type Agent,
  type Room as RoomType,
  type RoomMessage,
} from '../api/client'

export async function listRooms() {
  return ok(api.GET('/api/rooms'))
}

async function history(room: string) {
  const messages: RoomMessage[] = []
  for (;;) {
    const page = await ok(
      api.GET('/api/rooms/{room}/messages', {
        params: {
          path: { room },
          query: { after: messages.at(-1)?.sequence ?? 0 },
        },
      }),
    )
    messages.push(...page)
    if (page.length < 100) return messages
  }
}

/** Applies message snapshots by id, in room order. */
function merge(known: RoomMessage[], incoming: RoomMessage[]) {
  const byId = new Map(known.map((message) => [message.id, message]))
  for (const message of incoming) byId.set(message.id, message)
  return [...byId.values()].sort((a, b) => a.sequence - b.sequence)
}

const routing: Record<string, string> = {
  routing: 'Choosing an owner…',
  no_action: 'The coordinator found nothing for an agent to do.',
  failed: 'Routing failed. Mention an agent with @name to address it directly.',
}
const delivery: Record<string, string> = {
  queued: 'queued',
  running: 'working…',
  waiting: 'waiting for a retry in its conversation',
  delegated: 'waiting on delegated work',
  complete: 'replied',
  failed: 'failed',
  cancelled: 'cancelled',
}

export function Room({
  room,
  agents,
  onChanged,
}: {
  room: RoomType
  agents: Agent[]
  onChanged: (room: RoomType) => void
}) {
  const [editing, setEditing] = useState(false)
  const [messages, setMessages] = useState<RoomMessage[]>([])
  const [loaded, setLoaded] = useState(false)
  const [readError, setReadError] = useState('')
  const [text, setText] = useState('')
  const [requestId, setRequestId] = useState('')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  useEffect(() => {
    const source = new EventSource(`/api/rooms/${room.id}/events`)
    // History loads once the stream is following, so nothing falls between the two; a
    // reconnect reloads it to catch messages that changed while disconnected.
    source.onopen = () => {
      setReadError('')
      history(room.id).then(
        (loaded) => {
          setMessages((current) => merge(current, loaded))
          setLoaded(true)
        },
        (error) => setReadError(failure(error)),
      )
    }
    source.onerror = () =>
      setReadError(
        source.readyState === EventSource.CLOSED
          ? 'Live updates stopped. Reload the page to reconnect.'
          : 'Reconnecting to live updates…',
      )
    source.addEventListener('message', (event) => {
      const message: RoomMessage = JSON.parse(event.data)
      setMessages((current) => merge(current, [message]))
    })
    return () => source.close()
  }, [room.id])
  async function send() {
    const problem = messageProblem(text)
    if (problem) return setError(problem)
    setBusy(true)
    setError('')
    const id = requestId || crypto.randomUUID()
    setRequestId(id)
    try {
      const posted = await ok(
        api.POST('/api/rooms/{room}/messages', {
          params: { path: { room: room.id } },
          body: { request_id: id, message: text },
        }),
      )
      setText('')
      setRequestId('')
      setMessages((current) =>
        current.some((known) => known.id === posted.id)
          ? current
          : merge(current, [posted]),
      )
    } catch (error) {
      if (rejectedInput(error)) setRequestId('')
      setError(failure(error))
    } finally {
      setBusy(false)
    }
  }
  return (
    <>
      <header className="conversation-header">
        <div>
          <h1>{room.name}</h1>
          <p className="muted">
            Room · {room.members.map((member) => member.name).join(', ')}
          </p>
        </div>
        {room.manage && (
          <div className="actions">
            <button className="secondary" onClick={() => setEditing(!editing)}>
              {editing ? 'Close members' : 'Edit members'}
            </button>
          </div>
        )}
      </header>
      {editing && (
        <Members
          room={room}
          agents={agents}
          onSaved={(changed) => {
            setEditing(false)
            onChanged(changed)
          }}
        />
      )}
      <section className="conversation" aria-label="Room">
        <div className="transcript" tabIndex={0} aria-label="Room messages">
          {!loaded && <p role="status">Loading room…</p>}
          {loaded && messages.length === 0 && (
            <div className="empty">
              <h2>Nothing here yet.</h2>
              <p>
                Mention members with @name to ask each of them, or write without
                a mention and the coordinator picks one owner.
              </p>
            </div>
          )}
          {messages.map((message) => (
            <article className="exchange" key={message.id}>
              <div className="human-message">
                <span className="speaker">{message.author}</span>
                <p>{message.message}</p>
              </div>
              {message.routing === 'continued' && (
                <p className="muted" role="status">
                  Added to {message.note}’s open task as steering.
                </p>
              )}
              {message.routing === 'clarify' && (
                <div className="agent-message">
                  <span className="speaker">Coordinator</span>
                  <p>{message.note}</p>
                </div>
              )}
              {routing[message.routing] && (
                <p className="muted" role="status">
                  {routing[message.routing]}
                </p>
              )}
              {message.deliveries.map((item) => (
                <div className="agent-message" key={item.turn}>
                  <span className="speaker">
                    {item.agent} · {delivery[item.status] ?? item.status}
                  </span>
                  {item.response && <p>{item.response}</p>}
                  {item.status === 'failed' && item.error && (
                    <p className="muted">{item.error.replaceAll('_', ' ')}</p>
                  )}
                </div>
              ))}
            </article>
          ))}
        </div>
        {readError && <p role="alert">{readError}</p>}
        <form
          className="composer"
          onSubmit={(event) => {
            event.preventDefault()
            void send()
          }}
        >
          <label htmlFor="room-message">Message the room</label>
          <textarea
            id="room-message"
            value={text}
            readOnly={Boolean(requestId)}
            onChange={(event) => setText(event.target.value)}
            rows={3}
            maxLength={16384}
            required
            placeholder="@name addresses a member; otherwise one owner is chosen"
          />
          <div className="composer-bottom">
            <span className="muted">
              Each mentioned agent gets its own copy and answers on its own.
            </span>
            <button disabled={busy || !text.trim()}>
              {busy ? 'Sending…' : requestId ? 'Retry send' : 'Send'}
            </button>
          </div>
          {error && <p role="alert">{error}</p>}
        </form>
      </section>
    </>
  )
}

function AgentChoices({
  agents,
  chosen,
  onChange,
}: {
  agents: Pick<Agent, 'id' | 'name'>[]
  chosen: string[]
  onChange: (chosen: string[]) => void
}) {
  return (
    <fieldset>
      <legend>Agents</legend>
      <p className="muted">
        Agents in a room can delegate to and message each other.
      </p>
      {agents.map((agent) => (
        <label key={agent.id}>
          <input
            type="checkbox"
            checked={chosen.includes(agent.id)}
            onChange={(event) =>
              onChange(
                event.target.checked
                  ? [...chosen, agent.id]
                  : chosen.filter((id) => id !== agent.id),
              )
            }
          />{' '}
          {agent.name}
        </label>
      ))}
    </fieldset>
  )
}

/** Replaces the member set; removed agents keep the work already given to them. */
function Members({
  room,
  agents,
  onSaved,
}: {
  room: RoomType
  agents: Agent[]
  onSaved: (room: RoomType) => void
}) {
  const [chosen, setChosen] = useState(room.members.map((member) => member.id))
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  // Members this person cannot see as agents still show, so saving never drops them silently.
  const known = new Set(agents.map((agent) => agent.id))
  const listed = [
    ...agents,
    ...room.members.filter((member) => !known.has(member.id)),
  ]
  return (
    <section className="settings">
      <form
        onSubmit={async (event) => {
          event.preventDefault()
          setBusy(true)
          setError('')
          try {
            onSaved(
              await ok(
                api.PUT('/api/rooms/{room}/members', {
                  params: { path: { room: room.id } },
                  body: { agents: chosen },
                }),
              ),
            )
          } catch (error) {
            setError(failure(error))
          } finally {
            setBusy(false)
          }
        }}
      >
        <AgentChoices agents={listed} chosen={chosen} onChange={setChosen} />
        {error && <p role="alert">{error}</p>}
        <button disabled={busy || chosen.length === 0}>
          {busy ? 'Saving…' : 'Save members'}
        </button>
      </form>
    </section>
  )
}

export function NewRoom({
  agents,
  onCreated,
  onCancel,
}: {
  agents: Agent[]
  onCreated: (room: RoomType) => void
  onCancel: () => void
}) {
  const [name, setName] = useState('')
  const [members, setMembers] = useState<string[]>([])
  const [requestId] = useState(() => crypto.randomUUID())
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  return (
    <section className="settings">
      <h2>Create a room</h2>
      <form
        onSubmit={async (event) => {
          event.preventDefault()
          setBusy(true)
          setError('')
          try {
            onCreated(
              await ok(
                api.POST('/api/rooms', {
                  body: { request_id: requestId, name, agents: members },
                }),
              ),
            )
          } catch (error) {
            setError(failure(error))
          } finally {
            setBusy(false)
          }
        }}
      >
        <label htmlFor="room-name">Name</label>
        <input
          id="room-name"
          value={name}
          onChange={(event) => setName(event.target.value)}
          maxLength={128}
          required
        />
        <AgentChoices agents={agents} chosen={members} onChange={setMembers} />
        {error && <p role="alert">{error}</p>}
        <div className="actions">
          <button disabled={busy || !name.trim() || members.length === 0}>
            {busy ? 'Creating…' : 'Create room'}
          </button>
          <button type="button" className="secondary" onClick={onCancel}>
            Cancel
          </button>
        </div>
      </form>
    </section>
  )
}
