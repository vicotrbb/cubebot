import { useEffect, useState } from 'react'
import { api, ApiError, failure, ok, type Skill } from '../../api/client'
import { Alert, Field } from '../../components/primitives'

type File = { path: string; content: string }

const TEMPLATE = `---
name: my-skill
description: What this skill does and when to use it.
---
1. First step.
2. How to verify it worked.
`

export function SkillEditor({
  skill,
  onDone,
}: {
  skill: Skill | null
  onDone: () => void
}) {
  const [revision, setRevision] = useState(skill?.revision ?? 0)
  const [files, setFiles] = useState<File[]>(
    skill ? [] : [{ path: 'SKILL.md', content: TEMPLATE }],
  )
  const [author, setAuthor] = useState('')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  useEffect(() => {
    if (!skill) return
    void ok(
      api.GET('/api/skills/{skill}/revisions/{revision}', {
        params: { path: { skill: skill.id, revision } },
      }),
    )
      .then((found) => {
        setFiles(
          Object.entries(found.files).map(([path, content]) => ({
            path,
            content,
          })),
        )
        setAuthor(authorOf(found))
      })
      .catch((error) => setError(failure(error)))
  }, [skill, revision])
  function update(index: number, file: Partial<File>) {
    setFiles(files.map((old, at) => (at === index ? { ...old, ...file } : old)))
  }
  async function save() {
    setBusy(true)
    setError('')
    const body = Object.fromEntries(
      files.map((file) => [file.path, file.content]),
    )
    try {
      await ok(
        skill
          ? api.PUT('/api/skills/{skill}', {
              params: { path: { skill: skill.id } },
              body: { files: body, expected_revision: skill.revision },
            })
          : api.POST('/api/skills', { body: { files: body } }),
      )
      onDone()
    } catch (error) {
      setError(
        error instanceof ApiError && error.status === 409
          ? skill
            ? 'Someone published a newer revision first. Go back and open the skill again.'
            : 'A skill with this name already exists.'
          : failure(error),
      )
    } finally {
      setBusy(false)
    }
  }
  return (
    <form
      className="form skill-editor"
      onSubmit={(event) => {
        event.preventDefault()
        void save()
      }}
    >
      <p className="muted small">
        SKILL.md starts with front matter naming the skill and describing when
        to use it. Other files go under scripts/, references/ or assets/. Saving
        publishes a new revision; earlier revisions stay.
      </p>
      {skill && (
        <div className="actions">
          <Field label="Revision">
            <select
              value={revision}
              onChange={(event) => {
                setError('')
                setRevision(Number(event.target.value))
              }}
            >
              {Array.from({ length: skill.revision }, (_, at) => at + 1).map(
                (number) => (
                  <option key={number} value={number}>
                    {number}
                  </option>
                ),
              )}
            </select>
          </Field>
          <span className="faint small">{author}</span>
        </div>
      )}
      {files.map((file, index) => (
        <div key={index} className="skill-editor__file">
          <Field label="Path">
            <input
              value={file.path}
              required
              readOnly={file.path === 'SKILL.md' && index === 0}
              onChange={(event) => update(index, { path: event.target.value })}
            />
          </Field>
          <Field label="Content">
            <textarea
              rows={file.path === 'SKILL.md' ? 14 : 6}
              value={file.content}
              style={{ fontFamily: 'var(--mono)', fontSize: 'var(--t-sm)' }}
              onChange={(event) =>
                update(index, { content: event.target.value })
              }
            />
          </Field>
          {file.path !== 'SKILL.md' && (
            <button
              type="button"
              className="link-btn small"
              onClick={() => setFiles(files.filter((_, at) => at !== index))}
            >
              Remove file
            </button>
          )}
        </div>
      ))}
      <Alert>{error}</Alert>
      <div className="actions">
        <button className="btn btn--primary" disabled={busy}>
          {busy ? 'Publishing…' : skill ? 'Publish revision' : 'Publish skill'}
        </button>
        <button
          type="button"
          className="btn"
          onClick={() =>
            setFiles([...files, { path: 'scripts/', content: '' }])
          }
        >
          Add file
        </button>
        <button type="button" className="btn btn--ghost" onClick={onDone}>
          Cancel
        </button>
      </div>
    </form>
  )
}

function authorOf(revision: {
  dream_run?: string | null
  agent?: string | null
  human?: string | null
}) {
  if (revision.dream_run) return 'proposed by a dream'
  if (revision.agent) return `written by ${revision.agent}`
  return revision.human ? 'written by a person' : 'ships with Cubebot'
}
