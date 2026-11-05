import { Code2, Eye, Maximize2, X } from 'lucide-react'
import { useEffect, useRef, useState, type ReactNode } from 'react'
import { CodeBlock } from '../../components/CodeBlock'
import { CopyButton } from '../../components/primitives'
import '../files/viewer.css'

export function ViewSwitch({
  code,
  onChange,
}: {
  code: boolean
  onChange: (code: boolean) => void
}) {
  return (
    <div className="view-switch" role="group" aria-label="View">
      <button
        type="button"
        aria-pressed={!code}
        onClick={() => onChange(false)}
      >
        <Eye aria-hidden="true" />
        Preview
      </button>
      <button type="button" aria-pressed={code} onClick={() => onChange(true)}>
        <Code2 aria-hidden="true" />
        Code
      </button>
    </div>
  )
}

export function PreviewBlock({
  label,
  language,
  source,
  closed,
  preview,
}: {
  label: string
  language: string
  source: string
  closed: boolean
  preview: (full: boolean) => ReactNode
}) {
  const [code, setCode] = useState(false)
  const [expanded, setExpanded] = useState(false)
  const showCode = code || !closed
  return (
    <div className="preview">
      <div className="preview__head">
        <span className="preview__label">{label}</span>
        {!closed && <span className="faint small">Writing…</span>}
        <span className="preview__tools">
          {closed && <ViewSwitch code={code} onChange={setCode} />}
          <CopyButton text={source} label={`Copy ${label}`} />
          <button
            type="button"
            className="btn btn--ghost btn--icon btn--sm"
            aria-label={`Expand ${label}`}
            title="Expand"
            onClick={() => setExpanded(true)}
          >
            <Maximize2 />
          </button>
        </span>
      </div>
      <div className="preview__body">
        {showCode ? (
          <CodeBlock
            bare
            numbers
            code={source}
            language={language}
            maxHeight={520}
          />
        ) : (
          preview(false)
        )}
      </div>
      {expanded && (
        <Expanded
          label={label}
          code={showCode}
          onCode={setCode}
          closed={closed}
          source={source}
          language={language}
          preview={preview}
          onClose={() => setExpanded(false)}
        />
      )}
    </div>
  )
}

function Expanded({
  label,
  code,
  onCode,
  closed,
  source,
  language,
  preview,
  onClose,
}: {
  label: string
  code: boolean
  onCode: (code: boolean) => void
  closed: boolean
  source: string
  language: string
  preview: (full: boolean) => ReactNode
  onClose: () => void
}) {
  const ref = useRef<HTMLDialogElement>(null)
  useEffect(() => {
    const dialog = ref.current
    if (dialog && !dialog.open) dialog.showModal()
  }, [])
  return (
    <dialog
      ref={ref}
      className="viewer"
      aria-label={label}
      onClose={onClose}
      onClick={(event) => {
        if (event.target === event.currentTarget) onClose()
      }}
    >
      <div className="viewer__sheet">
        <header className="viewer__head">
          <div className="viewer__title">
            <h2>{label}</h2>
          </div>
          {closed && <ViewSwitch code={code} onChange={onCode} />}
          <CopyButton text={source} label={`Copy ${label}`} />
          <button
            type="button"
            className="btn btn--ghost btn--icon"
            aria-label="Close"
            onClick={onClose}
          >
            <X />
          </button>
        </header>
        <div className="viewer__body" tabIndex={0}>
          {code ? (
            <CodeBlock bare numbers code={source} language={language} />
          ) : (
            preview(true)
          )}
        </div>
      </div>
    </dialog>
  )
}
