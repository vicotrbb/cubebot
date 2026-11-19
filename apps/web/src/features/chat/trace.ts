import type { Turn } from '../../api/client'
import type { components } from '../../api/schema'

export type Trace = components['schemas']['Trace']
type Event = Trace['events'][number]
type Payload = Event['payload']
type Progress = Turn['terminal'][number]
type Stage = components['schemas']['ModelStage']

export type Step =
  | {
      kind: 'model'
      at: number
      stage: Stage
      input: number
      cached: number
      output: number
      cost: number | null
    }
  | { kind: 'tool'; at: number; took: number | null; tool: Progress }
  | {
      kind: 'review'
      at: number
      outcome: components['schemas']['ReviewOutcome']
    }
  | { kind: 'steered'; at: number; message: string }
  | { kind: 'summary'; at: number; text: string }
  | { kind: 'error'; at: number; code: string }

export type Pulled =
  | { kind: 'memory'; query: string; found: { id: string; content: string }[] }
  | { kind: 'kb_search'; query: string; found: { id: string; title: string }[] }
  | { kind: 'kb_page'; id: string; title: string }
  | { kind: 'skill'; name: string }

export type Totals = {
  calls: number
  tools: number
  input: number
  cached: number
  output: number
  cost: number
  unpriced: boolean
}

const PULLING = new Set(['search_memory', 'kb_search', 'kb_read', 'load_skill'])

function commandOf(name: string, input: unknown) {
  if (
    (name === 'bash' || name === 'terminal') &&
    typeof input === 'object' &&
    input !== null &&
    'command' in input &&
    typeof input.command === 'string'
  )
    return input.command
  return `${name} ${JSON.stringify(input)}`
}

function results(events: Event[]) {
  const done = new Map<string, Event>()
  for (const event of events)
    if (event.payload.kind === 'tool_result' && !done.has(event.payload.id))
      done.set(event.payload.id, event)
  return done
}

function toolStep(
  event: Event & { payload: Extract<Payload, { kind: 'tool_call' }> },
  result: Event | undefined,
  trace: Trace,
  at: number,
): Step {
  const call = event.payload
  const outcome =
    result?.payload.kind === 'tool_result' ? result.payload.outcome : null
  const running = trace.status === 'running' || trace.status === 'queued'
  return {
    kind: 'tool',
    at,
    took: result ? Date.parse(result.at) - Date.parse(event.at) : null,
    tool: {
      id: call.id,
      turn_id: trace.turn,
      name: call.name,
      command: commandOf(call.name, call.input),
      status: outcome ? outcome.status : running ? 'running' : 'uncertain',
      outcome,
      reobserved: false,
    },
  }
}

function stepOf(
  event: Event,
  trace: Trace,
  done: Map<string, Event>,
): Step | null {
  const at = Date.parse(event.at) - Date.parse(trace.created_at)
  const payload = event.payload
  switch (payload.kind) {
    case 'usage':
      return {
        kind: 'model',
        at,
        stage: payload.stage ?? 'reply',
        input: payload.input_tokens,
        cached: payload.cached_input_tokens ?? 0,
        output: payload.output_tokens,
        cost: event.cost_usd ?? null,
      }
    case 'tool_call':
      return toolStep({ ...event, payload }, done.get(payload.id), trace, at)
    case 'review':
      return { kind: 'review', at, outcome: payload.outcome }
    case 'steered':
      return { kind: 'steered', at, message: payload.message }
    case 'summary':
      return { kind: 'summary', at, text: payload.text }
    case 'error':
      return { kind: 'error', at, code: payload.code }
    default:
      return null
  }
}

export function stepsOf(trace: Trace): Step[] {
  const done = results(trace.events)
  return trace.events.flatMap((event) => stepOf(event, trace, done) ?? [])
}

export function totalsOf(steps: Step[]): Totals {
  const totals: Totals = {
    calls: 0,
    tools: 0,
    input: 0,
    cached: 0,
    output: 0,
    cost: 0,
    unpriced: false,
  }
  for (const step of steps) {
    if (step.kind === 'tool') totals.tools += 1
    if (step.kind !== 'model') continue
    totals.calls += 1
    totals.input += step.input
    totals.cached += step.cached
    totals.output += step.output
    totals.cost += step.cost ?? 0
    totals.unpriced ||= step.cost === null
  }
  return totals
}

function parsed(output: string | undefined): unknown {
  try {
    return JSON.parse(output ?? '')
  } catch {
    return null
  }
}

function records(value: unknown) {
  return Array.isArray(value)
    ? value.filter(
        (item): item is Record<string, unknown> =>
          typeof item === 'object' && item !== null,
      )
    : []
}

const text = (value: unknown) => (typeof value === 'string' ? value : '')

function outputOf(tool: Progress) {
  const outcome = tool.outcome
  return outcome && 'output' in outcome ? outcome.output : undefined
}

function pulledFrom(tool: Progress, input: Record<string, unknown>): Pulled {
  const found = records(parsed(outputOf(tool)))
  if (tool.name === 'search_memory')
    return {
      kind: 'memory',
      query: text(input.query),
      found: found.map((item) => ({
        id: text(item.id),
        content: text(item.content),
      })),
    }
  if (tool.name === 'kb_search')
    return {
      kind: 'kb_search',
      query: text(input.query),
      found: found.map((item) => ({
        id: text(item.id),
        title: text(item.title),
      })),
    }
  if (tool.name === 'kb_read') {
    const title = /"title":"((?:[^"\\]|\\.)*)"/.exec(outputOf(tool) ?? '')
    return {
      kind: 'kb_page',
      id: text(input.page),
      title: title ? text(parsed(`"${title[1]}"`)) : '',
    }
  }
  return { kind: 'skill', name: text(input.name) }
}

export function pulledOf(steps: Step[]): Pulled[] {
  return steps.flatMap((step) => {
    if (step.kind !== 'tool' || !PULLING.has(step.tool.name)) return []
    const input = parsed(step.tool.command.slice(step.tool.name.length + 1))
    return [
      pulledFrom(
        step.tool,
        typeof input === 'object' && input !== null
          ? (input as Record<string, unknown>)
          : {},
      ),
    ]
  })
}
