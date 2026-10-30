import { Fragment, useEffect, useState } from 'react'
import {
  api,
  merge,
  messageProblem,
  ok,
  rejectedInput,
  failure,
  type Turn,
  type TurnEvent,
} from '../api/client'

async function history(id: string) {
  const messages: Turn[] = []
  for (;;) {
    const page = await ok(
      api.GET('/api/conversations/{conversation}/messages', {
        params: {
          path: { conversation: id },
          query: { after: messages.at(-1)?.sequence ?? 0 },
        },
      }),
    )
    messages.push(...page)
    if (page.length < 100) return messages
  }
}
function draft(key: string) {
  try {
    return sessionStorage.getItem(key) ?? ''
  } catch {
    return ''
  }
}
function save(key: string, value: string) {
  try {
    if (value) sessionStorage.setItem(key, value)
    else sessionStorage.removeItem(key)
  } catch {
    /* Draft remains in the current page when browser storage is unavailable. */
  }
}
function stateLabel(message: Turn) {
  if (message.status === 'queued')
    return 'Queued. Waiting for the agent to become available.'
  if (message.status === 'running') return 'Agent is responding…'
  if (message.status === 'cancelled')
    return 'Cancelled. Workspace changes may already have occurred.'
  if (
    message.status === 'waiting' &&
    message.terminal.some((tool) => tool.status === 'uncertain')
  )
    return 'Terminal effects are uncertain. Stop this turn, then ask the agent to inspect the workspace in a new message.'
  if (message.status === 'waiting')
    return `Waiting: ${(message.error ?? 'execution interrupted').replaceAll('_', ' ')}. Retry when ready.`
  return ''
}

export function Chat({ id, accountId }: { id: string; accountId: string }) {
  const key = `cubebot:${accountId}:${id}`
  const [messages, setMessages] = useState<Turn[]>([])
  const [streaming, setStreaming] = useState<Record<string, string>>({})
  const [text, setText] = useState(() => draft(key))
  const [pending, setPending] = useState(() => draft(`${key}:request`))
  const [error, setError] = useState('')
  const [readError, setReadError] = useState('')
  const [busy, setBusy] = useState(false)
  const [loaded, setLoaded] = useState(false)
  useEffect(() => {
    const source = new EventSource(`/api/conversations/${id}/events`)
    // History loads once the stream is following, so nothing falls between the two; a
    // reconnect reloads it to catch turns that changed without an event.
    source.onopen = () => {
      setReadError('')
      history(id).then(
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
    source.addEventListener('turn', (event) => {
      const turn: Turn = JSON.parse(event.data)
      setMessages((current) => merge(current, [turn]))
      // shortcut: a retried turn replayed on connect shows its interrupted run's text first;
      // record run boundaries on events if that confuses people.
      if (turn.status !== 'running')
        setStreaming((current) =>
          Object.fromEntries(
            Object.entries(current).filter(([key]) => key !== turn.id),
          ),
        )
    })
    source.addEventListener('turn_event', (event) => {
      const { turn_id, payload }: TurnEvent = JSON.parse(event.data)
      if (payload.kind === 'text_delta')
        setStreaming((current) => ({
          ...current,
          [turn_id]: (current[turn_id] ?? '') + payload.text,
        }))
    })
    return () => source.close()
  }, [id])
  async function send() {
    const problem = messageProblem(text)
    if (problem) {
      setPending('')
      save(`${key}:request`, '')
      setError(problem)
      return
    }
    setBusy(true)
    setError('')
    const requestId = pending || crypto.randomUUID()
    setPending(requestId)
    save(`${key}:request`, requestId)
    save(key, text)
    try {
      const turn = await ok(
        api.POST('/api/conversations/{conversation}/messages', {
          params: { path: { conversation: id } },
          body: { request_id: requestId, message: text },
        }),
      )
      setText('')
      setPending('')
      save(key, '')
      save(`${key}:request`, '')
      // The stream may already hold a newer state of this turn.
      setMessages((current) =>
        current.some((known) => known.id === turn.id)
          ? current
          : merge(current, [turn]),
      )
    } catch (error) {
      if (rejectedInput(error)) {
        setPending('')
        save(`${key}:request`, '')
      }
      setError(failure(error))
    } finally {
      setBusy(false)
    }
  }
  return (
    <section className="conversation" aria-label="Conversation">
      <div className="transcript" tabIndex={0} aria-label="Message history">
        {!loaded && <p role="status">Loading history…</p>}
        {loaded && messages.length === 0 && (
          <div className="empty">
            <h2>Your agent is ready.</h2>
            <p>Send a message. Cubebot saves it before the agent begins.</p>
          </div>
        )}
        {messages.map((message) => (
          <Exchange
            key={message.id}
            conversation={id}
            message={message}
            streaming={streaming[message.id]}
          />
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
        <label htmlFor="message">Message</label>
        <textarea
          id="message"
          value={text}
          readOnly={Boolean(pending)}
          onChange={(event) => {
            setText(event.target.value)
            save(key, event.target.value)
          }}
          rows={3}
          maxLength={16384}
          required
          placeholder="Ask your agent…"
        />
        <div className="composer-bottom">
          <span className="muted">
            Accepted messages and replies are saved. Administrators can read
            conversations.
          </span>
          <button disabled={busy || !text.trim()}>
            {busy ? 'Sending…' : pending ? 'Retry send' : 'Send message'}
          </button>
        </div>
        {pending && !busy && (
          <p role="status">
            Delivery has not been confirmed. Retry uses the same request ID.
          </p>
        )}
        {error && <p role="alert">{error}</p>}
      </form>
    </section>
  )
}

function Retry({
  conversation,
  message,
}: {
  conversation: string
  message: Turn
}) {
  const [attempt, setAttempt] = useState<{ request_id: string } | null>(null)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  return (
    <>
      <button
        className="secondary"
        disabled={busy}
        onClick={async () => {
          setBusy(true)
          setError('')
          const input = attempt ?? { request_id: crypto.randomUUID() }
          setAttempt(input)
          try {
            await ok(
              api.POST(
                '/api/conversations/{conversation}/messages/{turn}/retry',
                {
                  params: { path: { conversation, turn: message.id } },
                  body: input,
                },
              ),
            )
            setAttempt(null)
          } catch (error) {
            setError(failure(error))
          } finally {
            setBusy(false)
          }
        }}
      >
        {busy ? 'Retrying…' : 'Retry response'}
      </button>
      {error && <p role="alert">{error}</p>}
    </>
  )
}

function Exchange({
  conversation,
  message,
  streaming,
}: {
  conversation: string
  message: Turn
  streaming: string | undefined
}) {
  return (
    <article className="exchange">
      <div className="human-message">
        <span className="speaker">You</span>
        <p>{message.message}</p>
      </div>
      <div className="agent-message">
        <span className="speaker">Agent</span>
        {message.response ? (
          <p>{message.response}</p>
        ) : (
          <>
            {streaming && <p>{streaming}</p>}
            <p className="muted" role="status">
              {stateLabel(message)}
            </p>
          </>
        )}
        {message.status === 'waiting' &&
          !message.terminal.some((tool) => tool.status === 'uncertain') && (
            <Retry conversation={conversation} message={message} />
          )}
        <TerminalResults message={message} />
        {['queued', 'running', 'waiting'].includes(message.status) && (
          <Stop conversation={conversation} message={message} />
        )}
      </div>
    </article>
  )
}
function TerminalResults({ message }: { message: Turn }) {
  return (
    <>
      {message.terminal.map((tool) => (
        <Fragment key={tool.id}>
          {tool.reobserved && (
            <p className="desktop-marker">
              Control returned · agent re-observed the screen
            </p>
          )}
          <details className="terminal-result" open={tool.status === 'running'}>
            <summary>Terminal · {tool.status}</summary>
            <pre>{tool.command}</pre>
            {tool.outcome?.status === 'exited' && (
              <p>Exit status: {tool.outcome.code ?? 'terminated by signal'}</p>
            )}
            {tool.outcome?.status === 'stopped' && (
              <p>Stopped: {tool.outcome.reason.replaceAll('_', ' ')}</p>
            )}
            {tool.status === 'uncertain' && (
              <p>
                Effects are uncertain. This command will not be repeated
                automatically. Inspect the workspace before continuing.
              </p>
            )}
            {tool.outcome && 'output' in tool.outcome && (
              <div>
                <pre>{tool.outcome.output}</pre>
              </div>
            )}
          </details>
        </Fragment>
      ))}
    </>
  )
}
/** The orchestrator delivers the cancel to the agent over its runtime channel. */
function Stop({
  conversation,
  message,
}: {
  conversation: string
  message: Turn
}) {
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  return (
    <>
      <button
        className="secondary"
        disabled={busy}
        onClick={async () => {
          setBusy(true)
          setError('')
          try {
            await ok(
              api.POST(
                '/api/conversations/{conversation}/messages/{turn}/cancel',
                { params: { path: { conversation, turn: message.id } } },
              ),
            )
          } catch (error) {
            setError(failure(error))
          } finally {
            setBusy(false)
          }
        }}
      >
        {busy ? 'Stopping…' : 'Stop'}
      </button>
      {error && <p role="alert">{error}</p>}
    </>
  )
}
