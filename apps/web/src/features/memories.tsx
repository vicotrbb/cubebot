import { useEffect, useState } from 'react'
import {
  api,
  ApiError,
  failure,
  ok,
  type Agent,
  type Memory,
} from '../api/client'

export function Memories({
  agent,
  onClose,
}: {
  agent: Agent
  onClose: () => void
}) {
  const [items, setItems] = useState<Memory[]>([])
  const [query, setQuery] = useState('')
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState('')
  const path = { params: { path: { agent: agent.id } } }
  async function load(search = query) {
    setLoading(true)
    try {
      setItems(await searchMemories(agent.id, search))
    } finally {
      setLoading(false)
    }
  }
  /** Runs a change, then shows the current memories; a conflict shows the newer revision. */
  async function change(action: () => Promise<unknown>) {
    setError('')
    try {
      await action()
    } catch (error) {
      setError(
        error instanceof ApiError && error.status === 409
          ? 'Someone changed this memory first. The current version is shown below.'
          : failure(error),
      )
    }
    await load().catch((error) => setError(failure(error)))
  }
  useEffect(() => {
    void searchMemories(agent.id, '')
      .then(setItems)
      .catch((error) => setError(failure(error)))
      .finally(() => setLoading(false))
  }, [agent.id])
  return (
    <section>
      <header className="conversation-header">
        <div>
          <h1>{agent.name}: memories</h1>
          <p className="muted">
            The agent searches these in every conversation it works in. Deleting
            one removes it from search; its revision history is kept.
          </p>
        </div>
        <button className="secondary" onClick={onClose}>
          Back to conversation
        </button>
      </header>
      <form
        className="memory-search"
        role="search"
        onSubmit={(event) => {
          event.preventDefault()
          setError('')
          void load().catch((error) => setError(failure(error)))
        }}
      >
        <label>
          Search memories
          <input
            value={query}
            maxLength={1024}
            onChange={(event) => setQuery(event.target.value)}
          />
        </label>
        <button className="secondary">Search</button>
      </form>
      <form
        onSubmit={(event) => {
          event.preventDefault()
          const form = event.currentTarget
          const content = String(new FormData(form).get('content'))
          void change(async () => {
            await ok(
              api.POST('/api/agents/{agent}/memories', {
                ...path,
                body: { content },
              }),
            )
            form.reset()
          })
        }}
      >
        <label>
          New memory
          <textarea name="content" rows={2} required maxLength={4096} />
        </label>
        <div className="actions">
          <button>Add memory</button>
        </div>
      </form>
      {error && <p role="alert">{error}</p>}
      {loading && <p role="status">Loading memories…</p>}
      {!loading && items.length === 0 && (
        <p className="muted">
          {query.trim() ? 'No memories match.' : 'No memories yet.'}
        </p>
      )}
      <ul className="memory-list">
        {items.map((memory) => (
          <MemoryItem
            key={`${memory.id}:${memory.revision}`}
            memory={memory}
            onSave={(content) =>
              change(() =>
                ok(
                  api.PUT('/api/agents/{agent}/memories/{memory}', {
                    params: { path: { agent: agent.id, memory: memory.id } },
                    body: { content, expected_revision: memory.revision },
                  }),
                ),
              )
            }
            onDelete={() =>
              change(() =>
                ok(
                  api.DELETE('/api/agents/{agent}/memories/{memory}', {
                    params: {
                      path: { agent: agent.id, memory: memory.id },
                      query: { expected_revision: memory.revision },
                    },
                  }),
                ),
              )
            }
          />
        ))}
      </ul>
    </section>
  )
}

function searchMemories(agent: string, query: string) {
  return ok(
    api.GET('/api/agents/{agent}/memories', {
      params: {
        path: { agent },
        query: query.trim() ? { query } : {},
      },
    }),
  )
}

function MemoryItem({
  memory,
  onSave,
  onDelete,
}: {
  memory: Memory
  onSave: (content: string) => Promise<void>
  onDelete: () => Promise<void>
}) {
  const [editing, setEditing] = useState(false)
  const [busy, setBusy] = useState(false)
  async function run(action: () => Promise<void>) {
    setBusy(true)
    await action()
    setBusy(false)
  }
  const origin = memory.edited_by_human
    ? 'edited by a person'
    : 'saved by the agent'
  return (
    <li className="memory">
      {editing ? (
        <form
          onSubmit={(event) => {
            event.preventDefault()
            const content = String(
              new FormData(event.currentTarget).get('content'),
            )
            void run(() => onSave(content))
          }}
        >
          <label>
            Memory
            <textarea
              name="content"
              rows={3}
              required
              maxLength={4096}
              defaultValue={memory.content}
            />
          </label>
          <div className="actions">
            <button disabled={busy}>{busy ? 'Saving…' : 'Save'}</button>
            <button
              type="button"
              className="secondary"
              onClick={() => setEditing(false)}
            >
              Cancel
            </button>
          </div>
        </form>
      ) : (
        <>
          <p>{memory.content}</p>
          <div className="memory-meta">
            <span className="muted">
              Revision {memory.revision}, {origin}
            </span>
            <button className="text-button" onClick={() => setEditing(true)}>
              Edit
            </button>
            <button
              className="text-button"
              disabled={busy}
              onClick={() => void run(onDelete)}
            >
              Delete
            </button>
          </div>
        </>
      )}
    </li>
  )
}
