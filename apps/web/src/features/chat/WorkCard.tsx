import {
  Camera,
  CircleAlert,
  CircleCheck,
  CircleX,
  Globe,
  KeyRound,
  Loader,
  Monitor,
  Terminal,
  Wrench,
} from 'lucide-react'
import type { Turn } from '../../api/client'

type Progress = Turn['terminal'][number]
type Outcome = NonNullable<Progress['outcome']>

const DESKTOP = ['computer', 'screenshot', 'vault_fill', 'request_login']
function Icon({ name }: { name: string }) {
  if (['bash', 'terminal'].includes(name)) return <Terminal />
  if (name.startsWith('browser_')) return <Globe />
  if (name === 'vault_fill' || name === 'request_login') return <KeyRound />
  if (name === 'screenshot') return <Camera />
  if (DESKTOP.includes(name)) return <Monitor />
  return <Wrench />
}

function title(tool: Progress) {
  if (['bash', 'terminal'].includes(tool.name)) return 'Terminal'
  if (tool.name === 'computer') return 'Computer'
  return tool.name.replaceAll('_', ' ')
}

/** The card's overall mark: the tool's status, sharpened by a non-zero exit or a stop. */
function statusOf(tool: Progress) {
  const outcome = tool.outcome
  if (tool.status === 'uncertain') return 'uncertain'
  if (outcome?.status === 'stopped') return 'stopped'
  if (
    outcome?.status === 'exited' &&
    outcome.code !== 0 &&
    outcome.code !== null
  )
    return 'failed'
  return tool.status
}

function Mark({ status }: { status: string }) {
  if (status === 'running') return <Loader className="spin" />
  if (status === 'uncertain') return <CircleAlert className="warn" />
  if (status === 'failed' || status === 'stopped')
    return <CircleX className="danger" />
  return <CircleCheck className="ok" />
}

/** One tool call: collapsed to a line, expanded to its command, output and outcome. */
export function WorkCard({ tool }: { tool: Progress }) {
  const status = statusOf(tool)
  return (
    <details className="work" open={tool.status === 'running'}>
      <summary className="work__summary">
        <span className="work__icon">
          <Icon name={tool.name} />
        </span>
        <span className="work__title">{title(tool)}</span>
        <span className="work__cmd">{firstLine(tool.command)}</span>
        <span className="work__mark" title={status}>
          <Mark status={status} />
        </span>
      </summary>
      <div className="work__body">
        <pre className="work__pre">{tool.command}</pre>
        {tool.status === 'uncertain' && (
          <p className="notice notice--danger small">
            Effects are uncertain. This command will not be repeated
            automatically. Inspect the workspace before continuing.
          </p>
        )}
        {tool.outcome && <OutcomeView outcome={tool.outcome} />}
      </div>
    </details>
  )
}

function OutcomeView({ outcome }: { outcome: Outcome }) {
  return (
    <>
      {outcome.status === 'exited' && (
        <p className="small muted">
          Exit status {outcome.code ?? 'terminated by signal'}
        </p>
      )}
      {outcome.status === 'stopped' && (
        <p className="small muted">
          Stopped: {outcome.reason.replaceAll('_', ' ')}
        </p>
      )}
      {'output' in outcome && outcome.output && (
        <pre className="work__pre">{outcome.output}</pre>
      )}
      {outcome.status === 'screenshot' && (
        <p className="small faint">
          Screenshot kept in the ledger; shown in the Desktop view, not here.
        </p>
      )}
    </>
  )
}

function firstLine(text: string) {
  const line = text.split('\n')[0] ?? ''
  return line.length > 120 ? `${line.slice(0, 120)}…` : line
}
