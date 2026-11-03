import { CornerDownLeft, Paperclip } from 'lucide-react'
import { useRef, useState, type ReactNode } from 'react'
import { failure, messageProblem, rejectedInput } from '../../api/client'
import { Alert } from '../../components/primitives'

function read(key: string) {
  try {
    return sessionStorage.getItem(key) ?? ''
  } catch {
    return ''
  }
}
function write(key: string, value: string) {
  try {
    if (value) sessionStorage.setItem(key, value)
    else sessionStorage.removeItem(key)
  } catch {
    /* The draft remains in the current page when browser storage is unavailable. */
  }
}

/**
 * The message box, Slack-shaped: Enter sends, Shift+Enter breaks a line. Drafts and an
 * unconfirmed request id persist per installation, person and conversation (ADR 0053), so a
 * lost acknowledgement retries with the same id instead of sending twice.
 */
export function Composer({
  draftKey,
  placeholder,
  label,
  note,
  send,
  disabled = false,
  attach,
  children,
}: {
  draftKey: string
  placeholder: string
  label: string
  note?: ReactNode
  send: (requestId: string, text: string) => Promise<void>
  disabled?: boolean
  attach?: () => void
  children?: ReactNode
}) {
  const [text, setText] = useState(() => read(draftKey))
  const [pending, setPending] = useState(() => read(`${draftKey}:request`))
  const [error, setError] = useState('')
  const [busy, setBusy] = useState(false)
  const area = useRef<HTMLTextAreaElement>(null)
  async function submit() {
    const problem = messageProblem(text)
    if (problem) {
      setPending('')
      write(`${draftKey}:request`, '')
      setError(problem)
      return
    }
    setBusy(true)
    setError('')
    const requestId = pending || crypto.randomUUID()
    setPending(requestId)
    write(`${draftKey}:request`, requestId)
    write(draftKey, text)
    try {
      await send(requestId, text)
      setText('')
      setPending('')
      write(draftKey, '')
      write(`${draftKey}:request`, '')
      area.current?.focus()
    } catch (error) {
      if (rejectedInput(error)) {
        setPending('')
        write(`${draftKey}:request`, '')
      }
      setError(failure(error))
    } finally {
      setBusy(false)
    }
  }
  return (
    <form
      className="composer"
      onSubmit={(event) => {
        event.preventDefault()
        void submit()
      }}
    >
      {children}
      <div className="composer__box">
        <label htmlFor={`composer-${draftKey}`} className="sr-only">
          {label}
        </label>
        <textarea
          ref={area}
          id={`composer-${draftKey}`}
          className="composer__input"
          value={text}
          readOnly={Boolean(pending) && busy}
          disabled={disabled}
          rows={1}
          maxLength={16384}
          placeholder={placeholder}
          onChange={(event) => {
            setText(event.target.value)
            write(draftKey, event.target.value)
            // A retry must resend the same text; edited text is a new request.
            setPending('')
            write(`${draftKey}:request`, '')
            event.target.style.height = 'auto'
            event.target.style.height = `${Math.min(event.target.scrollHeight, 240)}px`
          }}
          onKeyDown={(event) => {
            if (
              event.key === 'Enter' &&
              !event.shiftKey &&
              !event.nativeEvent.isComposing
            ) {
              event.preventDefault()
              if (!busy && text.trim()) void submit()
            }
          }}
        />
        <div className="composer__bar">
          {attach && (
            <button
              type="button"
              className="btn btn--ghost btn--icon btn--sm"
              aria-label="Attach a shared file"
              title="Attach a shared file"
              onClick={attach}
            >
              <Paperclip />
            </button>
          )}
          <span className="composer__note faint small">{note}</span>
          <button
            className="btn btn--primary btn--sm"
            disabled={busy || disabled || !text.trim()}
            aria-label={pending && !busy ? 'Retry send' : label}
          >
            {busy ? 'Sending…' : pending ? 'Retry send' : 'Send'}
            <CornerDownLeft />
          </button>
        </div>
      </div>
      {pending && !busy && (
        <p role="status" className="small muted">
          Delivery has not been confirmed. Retry uses the same request ID.
        </p>
      )}
      <Alert>{error}</Alert>
    </form>
  )
}
