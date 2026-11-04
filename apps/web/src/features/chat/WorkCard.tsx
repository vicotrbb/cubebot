import {
  Camera,
  ChevronRight,
  CircleAlert,
  CircleCheck,
  CircleX,
  FilePen,
  FilePlus2,
  FileText,
  Globe,
  KeyRound,
  Library,
  Loader,
  Monitor,
  Terminal,
  Wrench,
} from 'lucide-react'
import type { Turn } from '../../api/client'
import { CodeBlock } from '../../components/CodeBlock'
import { CopyButton } from '../../components/primitives'
import { diffLines, workDigest, workView, type WorkView } from './work'

type Progress = Turn['terminal'][number]
type Outcome = NonNullable<Progress['outcome']>

function Icon({ family, name }: { family: WorkView['family']; name: string }) {
  if (name === 'vault_fill' || name === 'request_login') return <KeyRound />
  if (name === 'screenshot') return <Camera />
  if (name === 'write_file') return <FilePlus2 />
  if (name === 'edit_file') return <FilePen />
  if (family === 'file') return <FileText />
  if (family === 'shell') return <Terminal />
  if (family === 'browser') return <Globe />
  if (family === 'desktop') return <Monitor />
  if (family === 'knowledge') return <Library />
  return <Wrench />
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

/** One tool call on the work log's rail: a sentence, opening to the file, diff or terminal behind
 *  it. A write in progress stays open, so people can read along. */
export function WorkCard({ tool }: { tool: Progress }) {
  const status = statusOf(tool)
  const view = workView(tool)
  return (
    <details
      className="work"
      data-status={status}
      open={tool.status === 'running'}
    >
      <summary className="work__summary">
        <span className="work__icon">
          <Icon family={view.family} name={tool.name} />
        </span>
        <span className="work__line">
          <span className="work__verb">{view.verb}</span>{' '}
          <span
            className={
              view.family === 'shell' || view.family === 'file'
                ? 'work__object work__object--code'
                : 'work__object'
            }
          >
            {view.object}
          </span>
          {view.where && <span className="work__where"> in {view.where}</span>}
        </span>
        {view.stat && <span className="work__stat">{view.stat}</span>}
        <span className="work__mark" title={status}>
          <Mark status={status} />
        </span>
      </summary>
      <div className="work__body">
        <WorkBody tool={tool} view={view} />
        {tool.status === 'uncertain' && (
          <p className="notice notice--danger small">
            Effects are uncertain. This command will not be repeated
            automatically. Inspect the workspace before continuing.
          </p>
        )}
      </div>
    </details>
  )
}

function WorkBody({ tool, view }: { tool: Progress; view: WorkView }) {
  const body = view.body
  if (body.kind === 'shell')
    return <TerminalView tool={tool} command={body.command} />
  if (body.kind === 'file')
    return (
      <>
        {body.code && (
          <CodeBlock
            code={body.code}
            language={body.language}
            name={body.path}
            numbers
            maxHeight={420}
          />
        )}
        {tool.outcome && !body.code && <OutcomeView outcome={tool.outcome} />}
      </>
    )
  if (body.kind === 'diff')
    return (
      <>
        <DiffView path={body.path} before={body.before} after={body.after} />
        {tool.outcome && failedOutcome(tool) && (
          <OutcomeView outcome={tool.outcome} />
        )}
      </>
    )
  return (
    <>
      {body.kind === 'fields' && body.fields.length > 0 && (
        <dl className="work__fields">
          {body.fields.map(([key, value]) => (
            <div key={key}>
              <dt>{key}</dt>
              <dd>{value}</dd>
            </div>
          ))}
        </dl>
      )}
      {tool.outcome && <OutcomeView outcome={tool.outcome} />}
    </>
  )
}

const failedOutcome = (tool: Progress) =>
  tool.outcome?.status !== 'exited' || tool.outcome.code !== 0

/** A command and what it printed, as the terminal it ran in looks: dark in either theme. */
function TerminalView({ tool, command }: { tool: Progress; command: string }) {
  const outcome = tool.outcome
  const printed = outcome && 'output' in outcome ? outcome.output : ''
  return (
    <div className="term">
      <pre className="term__command">
        <span aria-hidden="true">$ </span>
        {command}
      </pre>
      {printed && <pre className="term__output">{printed}</pre>}
      {tool.status === 'running' && (
        <span className="term__cursor" aria-label="Still running" />
      )}
      {outcome?.status === 'exited' && outcome.code !== 0 && (
        <p className="term__exit">
          Exit status {outcome.code ?? 'terminated by signal'}
        </p>
      )}
      {outcome?.status === 'stopped' && (
        <p className="term__exit">
          Stopped: {outcome.reason.replaceAll('_', ' ')}
        </p>
      )}
    </div>
  )
}

function DiffView({
  path,
  before,
  after,
}: {
  path: string
  before: string
  after: string
}) {
  return (
    <div className="codeblock">
      <div className="codeblock__head">
        <span className="codeblock__name">{path}</span>
        <CopyButton text={after} label="Copy new text" />
      </div>
      <div className="diff" tabIndex={0}>
        {diffLines(before, after).map((line, index) => (
          <div key={index} className="diff__line" data-sign={line.sign}>
            <span aria-hidden="true">{line.sign === '+' ? '+' : '−'}</span>
            <code>{line.line || ' '}</code>
          </div>
        ))}
      </div>
    </div>
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
  const actions = `${tools.length} ${tools.length === 1 ? 'action' : 'actions'}`
  const digest = workDigest(tools)
  const last = tools.at(-1)
  const now = last && workView(last)
  return (
    <details className="worklog" open={running}>
      <summary className="worklog__summary">
        <ChevronRight className="worklog__chevron" aria-hidden="true" />
        {running ? (
          <>
            <Loader className="spin worklog__spin" aria-hidden="true" />
            <span className="worklog__label worklog__now">
              {now ? `${now.verb} ${now.object}` : 'Working'}
            </span>
          </>
        ) : (
          <span className="worklog__label">Worked through {actions}</span>
        )}
        {digest && <span className="worklog__digest">{digest}</span>}
        {failed > 0 && <span className="worklog__failed">{failed} failed</span>}
        {uncertain > 0 && (
          <span className="worklog__uncertain">{uncertain} uncertain</span>
        )}
      </summary>
      <div className="worklog__list">{children}</div>
    </details>
  )
}
