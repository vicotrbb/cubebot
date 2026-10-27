import { useEffect, useState } from 'react'
import {
  array,
  messageProblem,
  rejectedInput,
  request,
  failure,
  turn,
  type Turn,
} from '../api/client'

async function history(id: string) {
  const messages: Turn[] = []
  let after = 0
  for (;;) {
    const page = array(
      await request(`/conversations/${id}/messages?after=${after}`),
      turn,
    )
    messages.push(...page)
    if (page.length < 100) return messages
    const last = page.at(-1)
    if (!last) return messages
    after = last.sequence
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
  if (message.status === 'retrying')
    return 'Retrying the selected model after a temporary provider error…'
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
  const [text, setText] = useState(() => draft(key))
  const [pending, setPending] = useState(() => draft(`${key}:request`))
  const [error, setError] = useState('')
  const [readError, setReadError] = useState('')
  const [busy, setBusy] = useState(false)
  const [loaded, setLoaded] = useState(false)
  useEffect(() => {
    let cancelled = false
    let timer: ReturnType<typeof setTimeout>
    async function poll() {
      try {
        const messages = await history(id)
        if (!cancelled) {
          setMessages(messages)
          setReadError('')
          setLoaded(true)
        }
      } catch (error) {
        if (!cancelled) setReadError(failure(error))
      }
      if (!cancelled) timer = setTimeout(poll, 2000)
    }
    void poll()
    return () => {
      cancelled = true
      clearTimeout(timer)
    }
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
      await request(`/conversations/${id}/messages`, {
        request_id: requestId,
        message: text,
      })
      setText('')
      setPending('')
      save(key, '')
      save(`${key}:request`, '')
      setMessages(await history(id))
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
            onChanged={async () => setMessages(await history(id))}
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
  onRetried,
}: {
  conversation: string
  message: Turn
  onRetried: () => Promise<void>
}) {
  const [attempt, setAttempt] = useState<{
    request_id: string
    expected_authority: string | null
  } | null>(null)
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
          const input = attempt ?? {
            request_id: crypto.randomUUID(),
            expected_authority: message.authority_id,
          }
          setAttempt(input)
          try {
            await request(
              `/conversations/${conversation}/messages/${message.id}/retry`,
              input,
            )
            setAttempt(null)
            await onRetried()
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
  onChanged,
}: {
  conversation: string
  message: Turn
  onChanged: () => Promise<void>
}) {
  return (
    <article className="exchange">
      <div className="human-message">
        <span className="speaker">You</span>
        <p>{message.message}</p>
      </div>
      <div className="agent-message">
        <span className="speaker">Agent</span>
        {message.response !== null ? (
          <p>{message.response}</p>
        ) : (
          <p className="muted" role="status">
            {stateLabel(message)}
          </p>
        )}
        {message.status === 'waiting' &&
          !message.terminal.some((tool) => tool.status === 'uncertain') && (
            <Retry
              conversation={conversation}
              message={message}
              onRetried={onChanged}
            />
          )}
        <TerminalResults message={message} />
        {['queued', 'running', 'retrying', 'waiting'].includes(
          message.status,
        ) && (
          <Stop
            conversation={conversation}
            message={message}
            onChanged={onChanged}
          />
        )}
      </div>
    </article>
  )
}
function TerminalResults({ message }: { message: Turn }) {
  return (
    <>
      {message.terminal.map((tool) => (
        <details
          className="terminal-result"
          key={tool.id}
          open={tool.status === 'running'}
        >
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
      ))}
    </>
  )
}
function Stop({
  conversation,
  message,
  onChanged,
}: {
  conversation: string
  message: Turn
  onChanged: () => Promise<void>
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
            await request(
              `/conversations/${conversation}/messages/${message.id}/cancel`,
              {},
            )
            await onChanged()
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
