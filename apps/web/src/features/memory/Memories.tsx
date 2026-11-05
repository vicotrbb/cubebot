import { Search } from 'lucide-react'
import { useEffect, useState } from 'react'
import { api, ApiError, failure, ok, type Memory } from '../../api/client'
import type { AgentSummary } from '../../api/types'
import { Alert, Empty, Skeleton } from '../../components/primitives'

export function Memories({
  agent,
  editable,
}: {
  agent: AgentSummary
  editable: boolean
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
    <>
      <section className="panel__section">
        <h2>Memories</h2>
        <p className="panel__lead">
          One pool the agent searches in every conversation and room it works
          in. Deleting a memory removes it from search; its history is kept.
        </p>
        <form
          className="actions"
          role="search"
          onSubmit={(event) => {
            event.preventDefault()
            setError('')
            void load().catch((error) => setError(failure(error)))
          }}
        >
          <input
            className="input"
            aria-label="Search memories"
            placeholder="Search memories"
            value={query}
            maxLength={1024}
            style={{ flex: 1 }}
            onChange={(event) => setQuery(event.target.value)}
          />
          <button className="btn btn--sm btn--icon" aria-label="Search">
            <Search />
          </button>
        </form>
        {editable && (
          <form
            className="form"
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
            <label className="field">
              <span>New memory</span>
              <textarea name="content" rows={2} required maxLength={4096} />
            </label>
            <div className="actions">
              <button className="btn btn--sm">Add memory</button>
            </div>
          </form>
        )}
        <Alert>{error}</Alert>
      </section>
      <section className="panel__section">
        {loading && <Skeleton lines={3} />}
        {!loading && items.length === 0 && (
          <Empty
            title={query.trim() ? 'No memories match.' : 'No memories yet.'}
          >
            The agent writes memories as it works, and a dream tidies them while
            it is idle.
          </Empty>
        )}
        <ul className="list">
          {items.map((memory) => (
            <MemoryItem
              key={`${memory.id}:${memory.revision}`}
              memory={memory}
              editable={editable}
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
    </>
  )
}

function searchMemories(agent: string, query: string) {
  return ok(
    api.GET('/api/agents/{agent}/memories', {
      params: { path: { agent }, query: query.trim() ? { query } : {} },
    }),
  )
}

function MemoryItem({
  memory,
  editable,
  onSave,
  onDelete,
}: {
  memory: Memory
  editable: boolean
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
  if (editing)
    return (
      <li className="memory">
        <form
          className="form"
          onSubmit={(event) => {
            event.preventDefault()
            const content = String(
              new FormData(event.currentTarget).get('content'),
            )
            void run(() => onSave(content)).then(() => setEditing(false))
          }}
        >
          <label className="field">
            <span>Memory</span>
            <textarea
              name="content"
              rows={3}
              required
              maxLength={4096}
              defaultValue={memory.content}
            />
          </label>
          <div className="actions">
            <button className="btn btn--primary btn--sm" disabled={busy}>
              {busy ? 'Saving…' : 'Save'}
            </button>
            <button
              type="button"
              className="btn btn--ghost btn--sm"
              onClick={() => setEditing(false)}
            >
              Cancel
            </button>
          </div>
        </form>
      </li>
    )
  return (
    <li className="memory">
      <p style={{ whiteSpace: 'pre-wrap' }}>{memory.content}</p>
      <div className="actions" style={{ marginTop: 4 }}>
        <span className="faint small">
          Revision {memory.revision}, {origin}
        </span>
        {editable && (
          <>
            <button
              type="button"
              className="link-btn small"
              onClick={() => setEditing(true)}
            >
              Edit
            </button>
            <button
              type="button"
              className="link-btn small"
              disabled={busy}
              onClick={() => void run(onDelete)}
            >
              Delete
            </button>
          </>
        )}
      </div>
    </li>
  )
}
