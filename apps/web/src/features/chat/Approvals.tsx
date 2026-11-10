import {
  Bot,
  ChevronRight,
  CircleCheck,
  CircleX,
  Hash,
  Loader,
  Network,
  PencilLine,
} from 'lucide-react'
import { useQueryClient } from '@tanstack/react-query'
import { useState } from 'react'
import { api, ApiError, type Turn } from '../../api/client'
import type { AgentSummary } from '../../api/types'
import { keys } from '../../app/queries'
import { Alert } from '../../components/primitives'
import { Details, type Input } from '../agent/CreationRequests'

type Tool = Turn['terminal'][number]
type Status = 'pending' | 'approving' | 'approved' | 'declined' | 'failed'
type Local = { status: Status; note?: string | undefined; error?: string }

const KINDS: Record<string, { kind: string; icon: typeof Bot; order: number }> =
  {
    agent_create: { kind: 'agent', icon: Bot, order: 0 },
    agent_update: { kind: 'change', icon: PencilLine, order: 1 },
    room_create: { kind: 'room', icon: Hash, order: 2 },
    team_create: { kind: 'team', icon: Network, order: 3 },
  }

const text = (value: unknown) => (typeof value === 'string' ? value : '')

function inputOf(tool: Tool): Input {
  try {
    const parsed: unknown = JSON.parse(tool.command.slice(tool.name.length + 1))
    return typeof parsed === 'object' && parsed ? (parsed as Input) : {}
  } catch {
    return {}
  }
}

function refusal(status: number) {
  if (status === 403)
    return 'Only someone allowed to make this can approve it; rooms and teams need an administrator.'
  if (status === 404)
    return 'Someone already settled this request. Reload to see the outcome.'
  return new ApiError(status).message
}

const FALLBACK = { kind: 'agent', icon: Bot, order: 0 }

export const isAsk = (tool: Tool) => !!tool.request && tool.name in KINDS

export function useApprovals(agent: AgentSummary) {
  const client = useQueryClient()
  const [local, setLocal] = useState<Record<string, Local>>({})
  const set = (id: string, next: Local) =>
    setLocal((all) => ({ ...all, [id]: next }))
  const statusOf = (tool: Tool): Local =>
    local[tool.request ?? ''] ?? {
      status: (tool.approval ?? 'pending') as Status,
      note: tool.approval_note ?? undefined,
    }
  async function approve(tool: Tool) {
    const id = tool.request
    if (!id) return false
    set(id, { status: 'approving' })
    const { error, response } = await api.POST(
      '/api/agents/{id}/creations/{request}/approve',
      { params: { path: { id: agent.id, request: id } } },
    )
    if (response.ok) {
      set(id, { status: 'approved' })
      for (const key of [keys.agents, keys.rooms, keys.teams])
        void client.invalidateQueries({ queryKey: key })
      return true
    }
    if (response.status === 409) {
      const note = (error as { error?: string } | undefined)?.error
      set(id, { status: 'failed', note })
      return true
    }
    set(id, { status: 'pending', error: refusal(response.status) })
    return false
  }
  async function decline(tool: Tool, reason: string) {
    const id = tool.request
    if (!id) return
    set(id, { status: 'approving' })
    const { response } = await api.POST(
      '/api/agents/{id}/creations/{request}/decline',
      {
        params: { path: { id: agent.id, request: id } },
        body: { reason },
      },
    )
    set(
      id,
      response.ok
        ? { status: 'declined', note: reason || undefined }
        : { status: 'pending', error: refusal(response.status) },
    )
  }
  async function approveAll(tools: Tool[]) {
    const ordered = [...tools].sort(
      (a, b) => (KINDS[a.name]?.order ?? 9) - (KINDS[b.name]?.order ?? 9),
    )
    for (const tool of ordered) if (!(await approve(tool))) return
  }
  return { statusOf, approve, decline, approveAll }
}

export type Approvals = ReturnType<typeof useApprovals>

export function ApprovalBar({
  agent,
  tools,
  approvals,
}: {
  agent: AgentSummary
  tools: Tool[]
  approvals: Approvals
}) {
  const waiting = tools.filter(
    (tool) => isAsk(tool) && approvals.statusOf(tool).status === 'pending',
  )
  const working = tools.some(
    (tool) => isAsk(tool) && approvals.statusOf(tool).status === 'approving',
  )
  if (waiting.length === 0 && !working) return null
  return (
    <div className="approval-bar" role="status">
      <p>
        {working && waiting.length === 0 ? (
          'Settling requests…'
        ) : (
          <>
            <strong>
              {waiting.length} {waiting.length === 1 ? 'request' : 'requests'}
            </strong>{' '}
            from {agent.name} {waiting.length === 1 ? 'waits' : 'wait'} for you.{' '}
            {agent.name} hears every outcome in one message once all are
            settled.
          </>
        )}
      </p>
      {waiting.length > 1 && (
        <button
          type="button"
          className="btn btn--primary btn--sm"
          disabled={working}
          onClick={() => void approvals.approveAll(waiting)}
        >
          {working ? 'Approving…' : `Approve all ${waiting.length}`}
        </button>
      )}
    </div>
  )
}

const DONE: Record<string, string> = {
  agent: 'Created',
  room: 'Created',
  team: 'Created',
  change: 'Changed',
}

function Mark({ local, kind }: { local: Local; kind: string }) {
  if (local.status === 'approving')
    return (
      <span className="ask__state" data-state="approving">
        <Loader className="spin" aria-hidden="true" /> Working
      </span>
    )
  if (local.status === 'approved')
    return (
      <span className="ask__state" data-state="approved">
        <CircleCheck aria-hidden="true" /> {DONE[kind]}
      </span>
    )
  if (local.status === 'declined')
    return (
      <span className="ask__state" data-state="declined">
        <CircleX aria-hidden="true" /> Declined
      </span>
    )
  return (
    <span className="ask__state" data-state="failed">
      <CircleX aria-hidden="true" /> Not made
    </span>
  )
}

export function AskRow({
  agent,
  tool,
  approvals,
  canSettle,
}: {
  agent: AgentSummary
  tool: Tool
  approvals: Approvals
  canSettle: boolean
}) {
  const { kind, icon: Icon } = KINDS[tool.name] ?? FALLBACK
  const input = inputOf(tool)
  const local = approvals.statusOf(tool)
  const pending = local.status === 'pending'
  const [open, setOpen] = useState(false)
  const [declining, setDeclining] = useState(false)
  const [reason, setReason] = useState('')
  const name = text(kind === 'change' ? input.agent : input.name)
  const verb = kind === 'change' ? 'Change the agent' : `Create the ${kind}`
  const panel = `ask-${tool.id}`
  return (
    <div className="ask" data-state={local.status}>
      <div className="ask__row">
        <span className="work__icon">
          <Icon />
        </span>
        <button
          type="button"
          className="ask__toggle"
          aria-expanded={open}
          aria-controls={panel}
          onClick={() => setOpen(!open)}
        >
          <span className="work__verb">{verb}</span>{' '}
          <span className="work__object">{name}</span>
          <ChevronRight className="ask__chevron" aria-hidden="true" />
        </button>
        {pending && canSettle ? (
          <span className="ask__actions">
            <button
              type="button"
              className="btn btn--ghost btn--sm"
              onClick={() => {
                setOpen(true)
                setDeclining(true)
              }}
            >
              Decline
            </button>
            <button
              type="button"
              className="btn btn--primary btn--sm"
              onClick={() => void approvals.approve(tool)}
            >
              Approve
            </button>
          </span>
        ) : pending ? (
          <span className="ask__state" data-state="pending">
            Waiting for a person
          </span>
        ) : (
          <Mark local={local} kind={kind} />
        )}
      </div>
      {local.error && (
        <div className="ask__body">
          <Alert>{local.error}</Alert>
        </div>
      )}
      {open && (
        <div className="ask__body" id={panel}>
          <Details kind={kind} input={input} />
          {local.note && !pending && (
            <p className="ask__note">
              {local.status === 'declined' ? 'Reason' : 'Why'}: {local.note}
            </p>
          )}
          {pending && declining && canSettle && (
            <form
              className="creation-request__decline"
              onSubmit={(event) => {
                event.preventDefault()
                void approvals.decline(tool, reason.trim())
              }}
            >
              <input
                className="input"
                aria-label="Reason for declining, sent to the agent"
                value={reason}
                maxLength={2048}
                autoFocus
                placeholder={`Tell ${agent.name} why (optional)`}
                onChange={(event) => setReason(event.target.value)}
              />
              <button type="submit" className="btn btn--sm btn--danger">
                Decline
              </button>
              <button
                type="button"
                className="btn btn--ghost btn--sm"
                onClick={() => setDeclining(false)}
              >
                Cancel
              </button>
            </form>
          )}
        </div>
      )}
    </div>
  )
}
