import { Braces, Download, FileText, Printer } from 'lucide-react'
import { useEffect, useState } from 'react'
import { createPortal } from 'react-dom'
import { Markdown } from '../features/chat/Markdown'
import { Menu } from './primitives'
import { fileName, json, markdown, save, type Sheet } from './transcript'
import './export.css'

function PrintSheet({
  sheet,
  time,
}: {
  sheet: Sheet
  time: (at: string) => string
}) {
  return (
    <article className="print-sheet">
      <header className="print-sheet__head">
        <h1>{sheet.title}</h1>
        {sheet.details.length > 0 && (
          <dl className="print-sheet__details">
            {sheet.details.map(([label, value]) => (
              <div key={label}>
                <dt>{label}</dt>
                <dd>{value}</dd>
              </div>
            ))}
          </dl>
        )}
      </header>
      {sheet.summary && (
        <section className="print-sheet__summary">
          <h2>Summary</h2>
          <Markdown text={sheet.summary} />
        </section>
      )}
      <section>
        <h2>Transcript</h2>
        {sheet.lines.length ? (
          <ol className="print-sheet__lines">
            {sheet.lines.map((line, index) => (
              <li key={index} data-kind={line.kind}>
                <p className="print-sheet__who">
                  <b>{line.speaker}</b>
                  {line.at && <time dateTime={line.at}>{time(line.at)}</time>}
                </p>
                <p className="print-sheet__text">{line.text}</p>
              </li>
            ))}
          </ol>
        ) : (
          <p>Nothing was said.</p>
        )}
      </section>
      <footer className="print-sheet__foot">
        Exported from Cubebot on{' '}
        {new Date().toLocaleString(undefined, {
          dateStyle: 'long',
          timeStyle: 'short',
        })}
      </footer>
    </article>
  )
}

function usePrint(sheet: Sheet | null, done: () => void) {
  useEffect(() => {
    if (!sheet) return
    const title = document.title
    document.title = fileName(sheet, 'pdf').replace(/\.pdf$/, '')
    const finish = () => {
      document.title = title
      done()
    }
    window.addEventListener('afterprint', finish, { once: true })
    const frame = requestAnimationFrame(() => window.print())
    return () => {
      cancelAnimationFrame(frame)
      window.removeEventListener('afterprint', finish)
      document.title = title
    }
  }, [sheet, done])
}

export function ExportMenu({
  sheet,
  time,
  label,
  text,
  className = 'btn btn--ghost btn--icon btn--sm',
}: {
  sheet: () => Sheet
  time: (at: string) => string
  label: string
  text?: string
  className?: string
}) {
  const [printing, setPrinting] = useState<Sheet | null>(null)
  const [done] = useState(() => () => setPrinting(null))
  usePrint(printing, done)
  const download = (format: 'markdown' | 'json') => {
    const made = sheet()
    void (format === 'markdown'
      ? save(fileName(made, 'md'), 'text/markdown', markdown(made, time))
      : save(fileName(made, 'json'), 'application/json', json(made)))
  }
  return (
    <>
      <Menu
        className="menu--right"
        trigger={
          <>
            <Download aria-hidden="true" />
            {text && <span className="export-text">{text}</span>}
          </>
        }
        triggerClassName={className}
        label={label}
      >
        <button
          type="button"
          className="menu__item"
          onClick={() => download('markdown')}
        >
          <FileText /> Markdown
        </button>
        <button
          type="button"
          className="menu__item"
          onClick={() => download('json')}
        >
          <Braces /> JSON
        </button>
        <button
          type="button"
          className="menu__item"
          onClick={() => setPrinting(sheet())}
        >
          <Printer /> PDF…
        </button>
      </Menu>
      {printing &&
        createPortal(
          <PrintSheet sheet={printing} time={time} />,
          document.body,
        )}
    </>
  )
}
