import { useQuery, useQueryClient } from '@tanstack/react-query'
import { Check } from 'lucide-react'
import { type ReactNode, useEffect, useRef } from 'react'
import { api, ok } from '../../api/client'
import type { components } from '../../api/schema'
import { Cube } from '../../components/Cube'
import { backupName } from '../platform/backups'
import './restore.css'

type Progress = components['schemas']['RestoreProgress']

const STEPS = [
  'Reading the backup',
  'Loading the database',
  'Fetching shared files',
  'Preparing agent workspaces',
]

/** While the orchestrator restores a backup, nothing else answers, so only its progress shows. */
export function RestoreGate({ children }: { children: ReactNode }) {
  const client = useQueryClient()
  const restore = useQuery({
    queryKey: ['restore'],
    queryFn: () => ok(api.GET('/api/restore')),
    refetchInterval: (query) =>
      query.state.data && query.state.data.state !== 'none' ? 2000 : false,
  })
  const state = restore.data?.state
  const was = useRef(state)
  useEffect(() => {
    // Everything asked during the restore was refused; ask again now the console answers.
    if (was.current && was.current !== 'none' && state === 'none')
      void client.invalidateQueries({
        predicate: (query) => query.queryKey[0] !== 'restore',
      })
    was.current = state
  }, [state, client])
  if (!restore.data || state === 'none') return children
  return <RestoreScreen progress={restore.data} />
}

function RestoreScreen({ progress }: { progress: Progress }) {
  const failed = progress.state === 'failed'
  const current = Math.max(0, STEPS.indexOf(progress.step))
  return (
    <main className="restore" aria-labelledby="restore-title">
      <Cube size={72} mood={failed ? 'idle' : 'working'} />
      <h1 id="restore-title">
        {failed ? 'The restore stopped' : 'Restoring Cubebot'}
      </h1>
      <p className="restore__from">
        {progress.backup
          ? `From the backup of ${backupName(progress.backup)}`
          : 'Finding the backup'}
      </p>
      <ol className="restore__steps">
        {STEPS.map((step, index) => {
          const state =
            index < current ? 'done' : index === current ? 'now' : 'next'
          return (
            <li
              key={step}
              data-state={failed && state === 'now' ? 'failed' : state}
              aria-current={state === 'now' ? 'step' : undefined}
            >
              <span className="restore__mark" aria-hidden="true">
                {state === 'done' && <Check />}
              </span>
              {step}
              {index === 2 && progress.total > 0 && (
                <span className="restore__count tabular">
                  {progress.done} of {progress.total}
                </span>
              )}
            </li>
          )
        })}
      </ol>
      {failed ? (
        <div className="restore__failure" role="alert">
          <p>{progress.error}</p>
          <p className="small">
            Fix the restore settings, then restart the orchestrator. A restore
            whose database already loaded resumes where it stopped.
          </p>
        </div>
      ) : (
        <p className="restore__note small" role="status">
          The console opens when the restore finishes. Each agent then fetches
          its own workspace before it takes work. Connections and sign-ins come
          back as they were and may have expired.
        </p>
      )}
      {progress.missing.length > 0 && (
        <details className="restore__missing">
          <summary className="small">
            This backup lacks {progress.missing.length}{' '}
            {progress.missing.length === 1 ? 'part' : 'parts'}
          </summary>
          <ul>
            {progress.missing.map((missing) => (
              <li key={missing.part} className="small">
                <strong>{missing.part}</strong>: {missing.reason}
              </li>
            ))}
          </ul>
        </details>
      )}
    </main>
  )
}
