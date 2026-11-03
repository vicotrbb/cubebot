import {
  Camera,
  ChevronRight,
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
        <span className="work__cmd">{brief(tool)}</span>
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

/**
 * Every tool call of one turn under a single line, so a long run of work folds away. It
 * stays open while the turn runs and folds once it settles; a reader can toggle it.
 */
export function WorkLog({
  tools,
  running,
  children,
}: {
  tools: Progress[]
  running: boolean
  children: React.ReactNode
}) {
  const count = (statuses: string[]) =>
    tools.filter((tool) => statuses.includes(statusOf(tool))).length
  const failed = count(['failed', 'stopped'])
  // Uncertain is not failed: the effect may have happened.
  const uncertain = count(['uncertain'])
  const kinds = [...new Set(tools.map((tool) => kindOf(tool.name)))]
  const actions = `${tools.length} ${tools.length === 1 ? 'action' : 'actions'}`
  return (
    <details className="worklog" open={running}>
      <summary className="worklog__summary">
        <ChevronRight className="worklog__chevron" aria-hidden="true" />
        <span className="worklog__label">
          {running
            ? `Working · ${actions} so far`
            : `Worked through ${actions}`}
        </span>
        <span className="worklog__kinds" aria-hidden="true">
          {kinds.map((kind) => (
            <span key={kind} className="work__icon">
              <Icon name={kind} />
            </span>
          ))}
        </span>
        {failed > 0 && <span className="worklog__failed">{failed} failed</span>}
        {uncertain > 0 && (
          <span className="worklog__uncertain">{uncertain} uncertain</span>
        )}
      </summary>
      <div className="worklog__list">{children}</div>
    </details>
  )
}

/** Collapses tools to the icon families the log shows. */
function kindOf(name: string) {
  if (name.startsWith('browser_')) return 'browser_navigate'
  return name
}

/** The call in a few words: its command line, or the gist of its JSON arguments. */
function brief(tool: Progress) {
  const command = tool.command.startsWith(`${tool.name} {`)
    ? tool.command.slice(tool.name.length + 1)
    : tool.command
  return clip(gist(command) ?? command.split('\n')[0] ?? '')
}

function gist(command: string): string | null {
  let input: unknown
  try {
    input = JSON.parse(command)
  } catch {
    return null
  }
  if (typeof input !== 'object' || input === null) return null
  const args = input as Record<string, unknown>
  if (Array.isArray(args.actions))
    return args.actions.map(action).join(' · ') || 'screenshot'
  if (typeof args.action === 'string') return action(args)
  for (const key of ['url', 'text', 'path', 'name', 'query'])
    if (typeof args[key] === 'string') return args[key] as string
  return JSON.stringify(args)
}

/** One computer-use step: "click 412, 80", "type “hello”", "keypress CTRL+L". */
function action(step: unknown) {
  const item = (step ?? {}) as Record<string, unknown>
  const kind = String(item.type ?? item.action ?? 'action')
  if (typeof item.text === 'string') return `${kind} “${item.text}”`
  if (Array.isArray(item.keys)) return `${kind} ${item.keys.join('+')}`
  if (typeof item.x === 'number' && typeof item.y === 'number')
    return `${kind} ${item.x}, ${item.y}`
  return kind
}

function clip(line: string) {
  return line.length > 120 ? `${line.slice(0, 120)}…` : line
}
