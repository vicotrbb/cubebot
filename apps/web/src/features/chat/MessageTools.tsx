import { Link } from '@tanstack/react-router'
import { Check, Copy, ScanSearch, Trash2 } from 'lucide-react'
import { marked } from 'marked'
import {
  useEffect,
  useRef,
  useState,
  type CSSProperties,
  type ReactNode,
} from 'react'
import { api, failure, ok } from '../../api/client'
import { Alert, confirmAction } from '../../components/primitives'

export type Part = 'message' | 'response'

const INTERACTIVE =
  'a, button, input, textarea, select, summary, label, pre, iframe, video, audio, img, [role="button"]'

export async function copyRich(markdown: string) {
  const html = marked.parse(markdown, { async: false, gfm: true })
  if (typeof ClipboardItem !== 'undefined' && navigator.clipboard?.write) {
    try {
      await navigator.clipboard.write([
        new ClipboardItem({
          'text/plain': new Blob([markdown], { type: 'text/plain' }),
          'text/html': new Blob([html], { type: 'text/html' }),
        }),
      ])
      return
    } catch {
      await copyPlain(markdown)
      return
    }
  }
  await copyPlain(markdown)
}

async function copyPlain(text: string) {
  if (navigator.clipboard) return navigator.clipboard.writeText(text)
  const area = document.createElement('textarea')
  area.value = text
  area.style.position = 'fixed'
  area.style.opacity = '0'
  document.body.append(area)
  area.select()
  document.execCommand('copy')
  area.remove()
}

export function Message({
  className,
  text,
  part,
  conversation,
  turn,
  deletable,
  who,
  onDeleted,
  children,
}: {
  className: string
  text: string
  part: Part
  conversation: string
  turn: string
  deletable: boolean
  who: string
  onDeleted: (part: Part) => void
  children: ReactNode
}) {
  const [open, setOpen] = useState(false)
  const [at, setAt] = useState(0)
  const [copied, setCopied] = useState(false)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  const self = useRef<HTMLDivElement>(null)
  useEffect(() => {
    if (!open) return
    const away = (event: PointerEvent) => {
      if (!self.current?.contains(event.target as Node)) setOpen(false)
    }
    document.addEventListener('pointerdown', away)
    return () => document.removeEventListener('pointerdown', away)
  }, [open])
  useEffect(() => {
    if (!copied) return
    const timer = window.setTimeout(() => setCopied(false), 1600)
    return () => window.clearTimeout(timer)
  }, [copied])
  const copy = async () => {
    setError('')
    try {
      await copyRich(text)
      setCopied(true)
    } catch (error) {
      setError(failure(error))
    }
  }
  const remove = async () => {
    const yes = await confirmAction({
      title: part === 'message' ? 'Delete your message?' : 'Delete this reply?',
      body: `It disappears from this conversation and ${who} will no longer see it. This cannot be undone.`,
      action: 'Delete',
    })
    if (!yes) return
    setBusy(true)
    setError('')
    try {
      await ok(
        api.DELETE('/api/conversations/{conversation}/messages/{turn}/{part}', {
          params: { path: { conversation, turn, part } },
        }),
      )
      onDeleted(part)
    } catch (error) {
      setError(failure(error))
      setBusy(false)
    }
  }
  return (
    <div
      ref={self}
      className={className}
      data-tools={open ? 'open' : undefined}
      style={open ? ({ '--tools-y': `${at}px` } as CSSProperties) : undefined}
      onClick={(event) => {
        const target = event.target as Element
        if (target.closest(INTERACTIVE)) return
        if (!window.getSelection()?.isCollapsed) return
        if (!window.matchMedia('(hover: none)').matches) return
        const box = event.currentTarget.getBoundingClientRect()
        setAt(Math.max(0, event.clientY - box.top - 56))
        setOpen((shown) => !shown)
      }}
    >
      {children}
      {text.trim() !== '' || deletable || part === 'response' ? (
        <div
          className="msg__tools"
          role="group"
          aria-label={part === 'message' ? 'Message actions' : 'Reply actions'}
        >
          {text.trim() !== '' && (
            <CopyTool copied={copied} onCopy={() => void copy()} />
          )}
          {part === 'response' && (
            <Link
              to="/agents/$agentId/trace/$turnId"
              params={{ agentId: conversation, turnId: turn }}
              className="msg__tool"
              aria-label="Trace: how this reply was made"
              title="Trace"
            >
              <ScanSearch aria-hidden="true" />
            </Link>
          )}
          {deletable && (
            <button
              type="button"
              className="msg__tool msg__tool--danger"
              aria-label="Delete"
              title="Delete"
              disabled={busy}
              onClick={() => void remove()}
            >
              <Trash2 aria-hidden="true" />
            </button>
          )}
          <span className="sr-only" aria-live="polite">
            {copied ? 'Copied to the clipboard' : ''}
          </span>
        </div>
      ) : null}
      {error && (
        <div className="msg__error">
          <Alert>{error}</Alert>
        </div>
      )}
    </div>
  )
}

function CopyTool({ copied, onCopy }: { copied: boolean; onCopy: () => void }) {
  const label = copied ? 'Copied' : 'Copy'
  return (
    <button
      type="button"
      className="msg__tool"
      aria-label={label}
      title={label}
      data-done={copied || undefined}
      onClick={onCopy}
    >
      {copied ? <Check aria-hidden="true" /> : <Copy aria-hidden="true" />}
    </button>
  )
}
