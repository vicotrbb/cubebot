import { CornerDownLeft, Paperclip } from 'lucide-react'
import {
  useLayoutEffect,
  useRef,
  useState,
  type ClipboardEvent,
  type KeyboardEvent,
  type ReactNode,
  type RefObject,
} from 'react'
import { failure, messageProblem, rejectedInput } from '../../api/client'
import { Alert } from '../../components/primitives'
import {
  MOST_FILES,
  Tray,
  useAttachments,
  useDropZone,
  type Attachments,
} from './ComposerFiles'
import { useMentions, type Mentionable } from './MentionMenu'
import { mentionSpans } from './mentions'

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

/** An unconfirmed request id persists with the draft, so a lost acknowledgement retries with the same id instead of sending twice. */
export function Composer({
  draftKey,
  placeholder,
  label,
  note,
  send,
  disabled,
  attachable,
  mentionables,
  children,
}: {
  draftKey: string
  placeholder: string
  label: string
  note?: ReactNode
  send: (requestId: string, text: string, files: string[]) => Promise<void>
  disabled?: boolean
  attachable?: boolean
  mentionables?: Mentionable[]
  children?: ReactNode
}) {
  const [text, setText] = useState(() => read(draftKey))
  const [pending, setPending] = useState(() => read(`${draftKey}:request`))
  const [error, setError] = useState('')
  const [busy, setBusy] = useState(false)
  const area = useRef<HTMLTextAreaElement>(null)
  const mirror = useRef<HTMLDivElement>(null)
  const names = namesOf(mentionables)
  function renew() {
    setPending('')
    write(`${draftKey}:request`, '')
  }
  const attachments = useAttachments(renew)
  const drop = useDropZone(attachments.add)
  function edit(value: string) {
    setText(value)
    write(draftKey, value)
    // A retry must resend the same text; edited text is a new request.
    renew()
  }
  const mentions = useMentions(mentionables, area, text, edit, draftKey)
  useLayoutEffect(() => {
    const element = area.current
    if (!element) return
    element.style.height = 'auto'
    element.style.height = `${Math.min(element.scrollHeight, 240)}px`
    if (mirror.current) {
      mirror.current.style.height = element.style.height
      mirror.current.scrollTop = element.scrollTop
    }
  }, [text])
  async function submit() {
    const problem = problemOf(text, attachments)
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
      await send(requestId, text, attachments.files)
      attachments.clear()
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
  const mirrored = names.length > 0
  const ready = !busy && sendable(text, attachments)
  const dropping = attachable && drop.dropping
  const retrying = Boolean(pending) && !busy
  return (
    <form
      className="composer"
      onSubmit={(event) => {
        event.preventDefault()
        void submit()
      }}
      {...(attachable ? drop.handlers : {})}
    >
      {children}
      <div className="composer__box" data-dropping={dropping || undefined}>
        {mentions.menu}
        <label htmlFor={`composer-${draftKey}`} className="sr-only">
          {label}
        </label>
        {mirrored && <Mirror mirror={mirror} names={names} text={text} />}
        <textarea
          ref={area}
          id={`composer-${draftKey}`}
          className={`composer__input${mirrored ? ' composer__input--mirrored' : ''}`}
          value={text}
          readOnly={Boolean(pending) && busy}
          disabled={disabled}
          rows={1}
          maxLength={16384}
          placeholder={placeholder}
          {...mentions.comboBox}
          onChange={(event) => {
            edit(event.target.value)
            mentions.track(event.target)
          }}
          onSelect={(event) => mentions.track(event.currentTarget)}
          onPaste={(event) => {
            if (attachable) pasted(event, attachments.add)
          }}
          onBlur={mentions.close}
          onScroll={(event) => {
            if (mirror.current)
              mirror.current.scrollTop = event.currentTarget.scrollTop
          }}
          onKeyDown={(event) => {
            if (!mentions.steer(event) && entered(event) && ready) void submit()
          }}
        />
        <Tray attachments={attachments} />
        {dropping && (
          <div className="composer__drop" aria-hidden="true">
            <Paperclip />
            Drop to attach, up to {MOST_FILES} files
          </div>
        )}
        <div className="composer__bar">
          {attachable && <Picker add={attachments.add} disabled={disabled} />}
          <span className="composer__note faint small">{note}</span>
          <SendButton
            label={label}
            busy={busy}
            retrying={retrying}
            blocked={disabled || !ready}
          />
        </div>
      </div>
      {retrying && (
        <p role="status" className="small muted">
          Delivery has not been confirmed. Retry uses the same request ID.
        </p>
      )}
      <Alert>{error || attachments.error}</Alert>
    </form>
  )
}

function namesOf(mentionables: Mentionable[] | undefined) {
  return mentionables?.map((member) => member.name) ?? []
}

function SendButton({
  label,
  busy,
  retrying,
  blocked,
}: {
  label: string
  busy: boolean
  retrying: boolean
  blocked: boolean | undefined
}) {
  const shown = retrying ? 'Retry send' : 'Send'
  return (
    <button
      className="btn btn--primary btn--sm"
      disabled={blocked}
      aria-label={retrying ? 'Retry send' : label}
    >
      {busy ? 'Sending…' : shown}
      <CornerDownLeft />
    </button>
  )
}

function Mirror({
  mirror,
  names,
  text,
}: {
  mirror: RefObject<HTMLDivElement | null>
  names: string[]
  text: string
}) {
  return (
    <div ref={mirror} className="composer__mirror" aria-hidden="true">
      {mentionSpans(`${text} `, names).map((span, index) =>
        span.mention ? (
          <span key={index} className="mention">
            {span.text}
          </span>
        ) : (
          span.text
        ),
      )}
    </div>
  )
}

function Picker({
  add,
  disabled,
}: {
  add: (files: File[]) => void
  disabled: boolean | undefined
}) {
  const picker = useRef<HTMLInputElement>(null)
  return (
    <>
      <button
        type="button"
        className="btn btn--ghost btn--icon btn--sm"
        aria-label="Attach files"
        title="Attach files"
        disabled={disabled}
        onClick={() => picker.current?.click()}
      >
        <Paperclip />
      </button>
      <input
        ref={picker}
        type="file"
        multiple
        hidden
        onChange={(event) => {
          add([...(event.currentTarget.files ?? [])])
          event.currentTarget.value = ''
        }}
      />
    </>
  )
}

function entered(event: KeyboardEvent<HTMLTextAreaElement>) {
  if (event.key !== 'Enter' || event.shiftKey || event.nativeEvent.isComposing)
    return false
  event.preventDefault()
  return true
}

function sendable(text: string, attachments: Attachments) {
  return (
    !attachments.blocked &&
    (Boolean(text.trim()) || attachments.files.length > 0)
  )
}

function problemOf(text: string, attachments: Attachments) {
  if (!text.trim() && attachments.files.length > 0) return null
  return messageProblem(text)
}

function pasted(
  event: ClipboardEvent<HTMLTextAreaElement>,
  add: (files: File[]) => void,
) {
  const files = [...event.clipboardData.files]
  if (files.length === 0 || event.clipboardData.getData('text/plain')) return
  event.preventDefault()
  add(files)
}
