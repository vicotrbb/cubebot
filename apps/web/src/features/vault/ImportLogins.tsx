import { FileUp } from 'lucide-react'
import { useState } from 'react'
import { api, failure, ok, type VaultItem } from '../../api/client'
import { notRetired, useAgents } from '../../app/queries'
import { Alert } from '../../components/primitives'
import { AccessChoice } from './InstallationVault'
import {
  browserLogins,
  type Login,
  SKIP_LABEL,
  type Skip,
} from './browserLogins'

type Read = {
  file: string
  logins: Login[]
  skipped: Partial<Record<Skip, number>>
}

const plural = (count: number, one: string) =>
  `${count} ${one}${count === 1 ? '' : 's'}`

const EXPORTS = [
  ['Chrome, Edge or Brave', 'Password Manager › Settings › Export passwords'],
  ['Firefox', 'about:logins › ••• › Export passwords'],
  ['Safari', 'File › Export › Passwords'],
] as const

export function ImportLogins({ onDone }: { onDone: () => void }) {
  const [read, setRead] = useState<Read | null>(null)
  const [error, setError] = useState('')
  async function choose(file: File | undefined) {
    if (!file) return
    setError('')
    const found = browserLogins(await file.text())
    if (!found)
      return setError(
        `${file.name} is not a password export: it has no url, username and password columns.`,
      )
    setRead({ file: file.name, ...found })
  }
  return read ? (
    <Review read={read} onBack={() => setRead(null)} onDone={onDone} />
  ) : (
    <div className="form vault-import">
      <p className="small muted">
        Export your saved passwords from the browser as a CSV file, then choose
        it here.
      </p>
      <dl className="vault-import__how">
        {EXPORTS.map(([browser, path]) => (
          <div key={browser}>
            <dt>{browser}</dt>
            <dd>{path}</dd>
          </div>
        ))}
      </dl>
      <label className="vault-import__drop">
        <FileUp aria-hidden="true" />
        <span>Choose the CSV file</span>
        <input
          type="file"
          accept=".csv,text/csv"
          onChange={(event) => {
            void choose(event.target.files?.[0])
            event.target.value = ''
          }}
        />
      </label>
      <Alert>{error}</Alert>
      <p className="small faint">
        The file is read in this browser and never uploaded; only the logins are
        sent, each sealed as it arrives. The export holds every password in
        plain text, so delete it once you are done.
      </p>
    </div>
  )
}

function Review({
  read,
  onBack,
  onDone,
}: {
  read: Read
  onBack: () => void
  onDone: () => void
}) {
  const agents = useAgents()
  const [access, setAccess] = useState<VaultItem['access']>({
    mode: 'all_agents',
  })
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  const [done, setDone] = useState<{ imported: number; skipped: number }>()
  const left = Object.entries(read.skipped) as [Skip, number][]
  const total = left.reduce((sum, [, count]) => sum + count, 0)
  async function save() {
    if (access.mode === 'explicit_grants' && access.agents.length === 0)
      return setError('Choose at least one agent, or allow all agents.')
    setBusy(true)
    setError('')
    try {
      setDone(
        await ok(
          api.POST('/api/vault/import', {
            body: { access, logins: read.logins },
          }),
        ),
      )
    } catch (caught) {
      setError(failure(caught))
    } finally {
      setBusy(false)
    }
  }
  if (done)
    return (
      <div className="form" role="status">
        <p>
          Imported {plural(done.imported, 'login')}.
          {done.skipped > 0 &&
            ` ${plural(done.skipped, 'login')} the vault already held ${done.skipped === 1 ? 'was' : 'were'} left as they were.`}
        </p>
        <div className="actions">
          <button type="button" className="btn btn--primary" onClick={onDone}>
            Done
          </button>
        </div>
      </div>
    )
  return (
    <div className="form vault-import">
      <p>
        {read.logins.length > 0
          ? `${plural(read.logins.length, 'login')} from ${read.file} can be filled by agents.`
          : `Nothing in ${read.file} can be filled by agents.`}
      </p>
      {left.length > 0 && (
        <div className="vault-import__left">
          <p className="small muted">
            {total} {total === 1 ? 'entry was' : 'entries were'} left out:
          </p>
          <ul className="small muted">
            {left.map(([reason, count]) => (
              <li key={reason}>
                {count} {SKIP_LABEL[reason][count === 1 ? 0 : 1]}
              </li>
            ))}
          </ul>
        </div>
      )}
      {read.logins.length > 0 && (
        <AccessChoice
          access={access}
          agents={notRetired(agents.data)}
          onChange={setAccess}
        />
      )}
      <Alert>{error}</Alert>
      <div className="actions">
        {read.logins.length > 0 && (
          <button
            type="button"
            className="btn btn--primary"
            disabled={busy}
            onClick={() => void save()}
          >
            {busy
              ? 'Importing…'
              : `Import ${plural(read.logins.length, 'login')}`}
          </button>
        )}
        <button type="button" className="btn btn--ghost" onClick={onBack}>
          Choose another file
        </button>
      </div>
    </div>
  )
}
