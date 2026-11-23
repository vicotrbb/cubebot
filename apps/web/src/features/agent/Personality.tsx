import { useQuery, useQueryClient } from '@tanstack/react-query'
import { useState } from 'react'
import { api, failure, ok } from '../../api/client'
import type { AgentSummary } from '../../api/types'
import { Cube, moodOf } from '../../components/Cube'
import { Alert, Skeleton, Status, when } from '../../components/primitives'

const LIMIT = 800

function Editor({
  agent,
  draft,
  busy,
  error,
  onDraft,
  onSave,
  onCancel,
}: {
  agent: AgentSummary
  draft: string
  busy: boolean
  error: string
  onDraft: (draft: string) => void
  onSave: () => void
  onCancel: () => void
}) {
  const length = [...draft].length
  const over = length > LIMIT
  return (
    <form
      className="form"
      onSubmit={(event) => {
        event.preventDefault()
        onSave()
      }}
    >
      <label className="personality__editor">
        <span className="sr-only">Personality</span>
        <textarea
          value={draft}
          onChange={(event) => onDraft(event.target.value)}
          aria-invalid={over || undefined}
          aria-describedby="personality-count"
          rows={9}
          autoFocus
          placeholder={`In ${agent.name}'s words: temperament, humor, what it cares about, how it writes.`}
        />
        <span
          id="personality-count"
          className="personality__count faint small"
          data-near={length > LIMIT - 80 || undefined}
        >
          {length} / {LIMIT}
        </span>
      </label>
      {over && (
        <p className="personality__over small" role="status">
          Keep it to {LIMIT} characters: temperament, voice and values. Facts,
          people and plans belong in memories.
        </p>
      )}
      <Alert>{error}</Alert>
      <div className="actions">
        <button className="btn btn--primary btn--sm" disabled={busy || over}>
          {busy ? 'Saving…' : 'Save'}
        </button>
        <button
          type="button"
          className="btn btn--sm"
          disabled={busy}
          onClick={onCancel}
        >
          Cancel
        </button>
      </div>
    </form>
  )
}

export function Personality({ agent }: { agent: AgentSummary }) {
  const key = ['agents', agent.id, 'personality']
  const client = useQueryClient()
  const read = useQuery({
    queryKey: key,
    queryFn: () =>
      ok(
        api.GET('/api/agents/{agent}/personality', {
          params: { path: { agent: agent.id } },
        }),
      ),
    retry: false,
  })
  const [draft, setDraft] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  const [notice, setNotice] = useState('')
  if (read.isError) return null
  const text = read.data?.personality ?? ''
  const save = async () => {
    if (draft === null) return
    setBusy(true)
    setError('')
    setNotice('')
    try {
      const saved = await ok(
        api.PUT('/api/agents/{agent}/personality', {
          params: { path: { agent: agent.id } },
          body: { personality: draft },
        }),
      )
      client.setQueryData(key, saved)
      setDraft(null)
      setNotice(
        'Saved. It carries your edit into its next turn and keeps growing from there.',
      )
    } catch (error) {
      setError(failure(error))
    } finally {
      setBusy(false)
    }
  }
  return (
    <section className="panel__section personality">
      <div className="personality__head">
        <h2>Personality</h2>
        {draft === null && read.data && (
          <button
            type="button"
            className="btn btn--sm"
            onClick={() => {
              setNotice('')
              setDraft(text)
            }}
          >
            {text ? 'Edit' : 'Write one'}
          </button>
        )}
      </div>
      <p className="panel__lead">
        {agent.name} writes this short sketch of itself and refines it while it
        dreams. It shapes how {agent.name} thinks, writes and takes initiative.
      </p>
      {!read.data ? (
        <Skeleton lines={3} />
      ) : draft !== null ? (
        <Editor
          agent={agent}
          draft={draft}
          busy={busy}
          error={error}
          onDraft={setDraft}
          onSave={() => void save()}
          onCancel={() => {
            setError('')
            setDraft(null)
          }}
        />
      ) : text ? (
        <figure className="personality__letter">
          <Cube
            agentId={agent.id}
            avatar={agent.avatar}
            mood={moodOf(agent)}
            size={36}
            className="personality__face"
          />
          <blockquote className="personality__text">{text}</blockquote>
          <figcaption className="personality__meta faint small">
            {read.data.edited_by
              ? `Edited by ${read.data.edited_by} ${when(read.data.updated_at)}`
              : `In ${agent.name}'s words, last grown ${when(read.data.updated_at)}`}
          </figcaption>
        </figure>
      ) : (
        <div className="personality__empty">
          <Cube
            agentId={agent.id}
            avatar={agent.avatar}
            mood="idle"
            size={36}
          />
          <p className="small muted">
            Nothing yet. {agent.name} writes its first version after it has done
            some work and rests, or you can give it a starting point.
          </p>
        </div>
      )}
      <Status>{notice}</Status>
    </section>
  )
}
