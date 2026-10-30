import { useEffect, useState } from 'react'
import {
  api,
  ApiError,
  failure,
  ok,
  type Agent,
  type InstalledSkill,
  type Skill,
} from '../api/client'

type File = { path: string; content: string }

const template = `---
name: my-skill
description: What this skill does and when to use it.
---
1. First step.
2. How to verify it worked.
`

/** The agent's installed skills and the installation's shared library. */
export function Skills({
  agent,
  onClose,
}: {
  agent: Agent
  onClose: () => void
}) {
  const [installed, setInstalled] = useState<InstalledSkill[]>([])
  const [library, setLibrary] = useState<Skill[]>([])
  const [editing, setEditing] = useState<Skill | 'new' | null>(null)
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState('')
  function apply([mine, all]: [InstalledSkill[], Skill[]]) {
    setInstalled(mine)
    setLibrary(all)
  }
  async function load() {
    apply(await listSkills(agent.id))
  }
  async function change(action: () => Promise<unknown>) {
    setError('')
    try {
      await action()
    } catch (error) {
      setError(failure(error))
    }
    await load().catch((error) => setError(failure(error)))
  }
  function install(skill: string, revision: number) {
    return change(() =>
      ok(
        api.PUT('/api/agents/{agent}/skills/{skill}', {
          params: { path: { agent: agent.id, skill } },
          body: { revision },
        }),
      ),
    )
  }
  useEffect(() => {
    void listSkills(agent.id)
      .then(([mine, all]) => {
        setInstalled(mine)
        setLibrary(all)
      })
      .catch((error) => setError(failure(error)))
      .finally(() => setLoading(false))
  }, [agent.id])
  if (editing)
    return (
      <SkillEditor
        skill={editing === 'new' ? null : editing}
        onDone={() => {
          setEditing(null)
          void load().catch((error) => setError(failure(error)))
        }}
      />
    )
  const pinned = new Map(installed.map((skill) => [skill.skill, skill]))
  return (
    <section>
      <header className="conversation-header">
        <div>
          <h1>{agent.name}: skills</h1>
          <p className="muted">
            The agent loads an installed skill when a task needs it. A new
            revision in the library does not change an installation until you
            update it.
          </p>
        </div>
        <div className="actions">
          <button className="secondary" onClick={() => setEditing('new')}>
            New skill
          </button>
          <button className="secondary" onClick={onClose}>
            Back to conversation
          </button>
        </div>
      </header>
      {error && <p role="alert">{error}</p>}
      {loading && <p role="status">Loading skills…</p>}
      <h2>Installed</h2>
      {!loading && installed.length === 0 && (
        <p className="muted">No skills installed.</p>
      )}
      <ul className="memory-list">
        {installed.map((skill) => (
          <li key={skill.skill} className="memory">
            <p>
              <strong>{skill.name}</strong>: {skill.description}
            </p>
            <div className="memory-meta">
              <span className="muted">
                Revision {skill.revision}
                {skill.current_revision > skill.revision &&
                  `, revision ${skill.current_revision} available`}
              </span>
              {skill.current_revision > skill.revision && (
                <button
                  className="text-button"
                  onClick={() =>
                    void install(skill.skill, skill.current_revision)
                  }
                >
                  Update
                </button>
              )}
              <button
                className="text-button"
                onClick={() =>
                  void change(() =>
                    ok(
                      api.DELETE('/api/agents/{agent}/skills/{skill}', {
                        params: {
                          path: { agent: agent.id, skill: skill.skill },
                        },
                      }),
                    ),
                  )
                }
              >
                Uninstall
              </button>
            </div>
          </li>
        ))}
      </ul>
      <h2>Library</h2>
      {!loading && library.length === 0 && (
        <p className="muted">No skills in the library yet.</p>
      )}
      <ul className="memory-list">
        {library.map((skill) => (
          <li key={skill.id} className="memory">
            <p>
              <strong>{skill.name}</strong>: {skill.description}
            </p>
            <div className="memory-meta">
              <span className="muted">Revision {skill.revision}</span>
              {!pinned.has(skill.id) && (
                <button
                  className="text-button"
                  onClick={() => void install(skill.id, skill.revision)}
                >
                  Install
                </button>
              )}
              <button className="text-button" onClick={() => setEditing(skill)}>
                Open
              </button>
            </div>
          </li>
        ))}
      </ul>
    </section>
  )
}

function listSkills(agent: string) {
  return Promise.all([
    ok(
      api.GET('/api/agents/{agent}/skills', {
        params: { path: { agent } },
      }),
    ),
    ok(api.GET('/api/skills')),
  ])
}

/** Shows any revision and saves edits as a new revision on top of the current one. */
function SkillEditor({
  skill,
  onDone,
}: {
  skill: Skill | null
  onDone: () => void
}) {
  const [revision, setRevision] = useState(skill?.revision ?? 0)
  const [files, setFiles] = useState<File[]>(
    skill ? [] : [{ path: 'SKILL.md', content: template }],
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
        setAuthor(
          found.dream_run ? 'proposed by a dream' : 'written by a person',
        )
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
    <section>
      <header className="conversation-header">
        <div>
          <h1>{skill ? skill.name : 'New skill'}</h1>
          <p className="muted">
            SKILL.md starts with front matter naming the skill and describing
            when to use it. Other files go under scripts/, references/ or
            assets/. Saving publishes a new revision; earlier revisions stay.
          </p>
        </div>
        <button className="secondary" onClick={onDone}>
          Back to skills
        </button>
      </header>
      {skill && (
        <div className="memory-meta">
          <label>
            Revision{' '}
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
          </label>
          <span className="muted">{author}</span>
        </div>
      )}
      <form
        onSubmit={(event) => {
          event.preventDefault()
          void save()
        }}
      >
        <ul className="memory-list">
          {files.map((file, index) => (
            <li key={index} className="memory">
              <label>
                Path
                <input
                  value={file.path}
                  required
                  readOnly={file.path === 'SKILL.md' && index === 0}
                  onChange={(event) =>
                    update(index, { path: event.target.value })
                  }
                />
              </label>
              <label>
                Content
                <textarea
                  rows={file.path === 'SKILL.md' ? 12 : 6}
                  value={file.content}
                  onChange={(event) =>
                    update(index, { content: event.target.value })
                  }
                />
              </label>
              {file.path !== 'SKILL.md' && (
                <button
                  type="button"
                  className="text-button"
                  onClick={() =>
                    setFiles(files.filter((_, at) => at !== index))
                  }
                >
                  Remove file
                </button>
              )}
            </li>
          ))}
        </ul>
        {error && <p role="alert">{error}</p>}
        <div className="actions">
          <button disabled={busy}>
            {busy
              ? 'Publishing…'
              : skill
                ? 'Publish revision'
                : 'Publish skill'}
          </button>
          <button
            type="button"
            className="secondary"
            onClick={() =>
              setFiles([...files, { path: 'scripts/', content: '' }])
            }
          >
            Add file
          </button>
        </div>
      </form>
    </section>
  )
}
