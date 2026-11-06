import { CircleAlert, Plus, X } from 'lucide-react'
import { useEffect, useRef, useState } from 'react'
import { Account } from './Account'
import { Mark } from './Mark'
import type { Catalog, Connection, Manifest } from './model'
import { Setup } from './Setup'

/** One connector's accounts and their setup, beside the catalog; the whole screen on a phone. */
export function Sheet({
  catalog,
  manifest,
  connections,
  all,
  focus,
  onClose,
}: {
  catalog: Catalog
  manifest: Manifest
  connections: Connection[]
  all: Connection[]
  focus: string | undefined
  onClose: () => void
}) {
  const ref = useRef<HTMLDialogElement>(null)
  const [expanded, setExpanded] = useState<string | null>(
    focus ?? (connections.length === 1 ? (connections[0]?.id ?? null) : null),
  )
  const [adding, setAdding] = useState<Connection | 'new' | null>(
    connections.length === 0 ? 'new' : null,
  )
  useEffect(() => {
    const dialog = ref.current
    if (dialog && !dialog.open) dialog.showModal()
  }, [])
  const done = (saved: Connection | null) => {
    setAdding(null)
    if (saved) setExpanded(saved.id)
  }
  return (
    <dialog
      ref={ref}
      className="sheet"
      aria-labelledby="sheet-title"
      onClose={onClose}
      onClick={(event) => {
        if (event.target === event.currentTarget) onClose()
      }}
    >
      <header className="sheet__head">
        <Mark id={manifest.id} size="lg" />
        <div className="sheet__title">
          <h2 id="sheet-title">{manifest.name}</h2>
          <p>{manifest.description}</p>
        </div>
        <button
          type="button"
          className="btn btn--ghost btn--icon"
          aria-label="Close"
          onClick={() => ref.current?.close()}
        >
          <X />
        </button>
      </header>
      <div className="sheet__body">
        {manifest.warning && (
          <p className="notice notice--paint sheet__warning">
            <CircleAlert aria-hidden="true" />
            <span>{manifest.warning}</span>
          </p>
        )}
        {connections.length > 0 && (
          <section className="sheet__section" aria-labelledby="accounts-title">
            <div className="sheet__section-head">
              <h3 id="accounts-title">
                {connections.length === 1
                  ? 'Account'
                  : `${connections.length} accounts`}
              </h3>
              {adding === null && (
                <button
                  type="button"
                  className="btn btn--sm"
                  onClick={() => setAdding('new')}
                >
                  <Plus /> Add account
                </button>
              )}
            </div>
            <ul className="accounts">
              {connections.map((connection) => (
                <Account
                  key={connection.id}
                  manifest={manifest}
                  connection={connection}
                  expanded={expanded === connection.id}
                  onToggle={() =>
                    setExpanded(
                      expanded === connection.id ? null : connection.id,
                    )
                  }
                  onSignInAgain={() => setAdding(connection)}
                />
              ))}
            </ul>
          </section>
        )}
        {adding !== null && (
          <Setup
            catalog={catalog}
            manifest={manifest}
            replacing={adding === 'new' ? null : adding}
            all={all}
            onDone={done}
            onCancel={
              connections.length > 0 ? () => setAdding(null) : undefined
            }
          />
        )}
      </div>
    </dialog>
  )
}
