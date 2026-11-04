import type { components } from '../../api/schema'

export type Task = components['schemas']['Task']
export type TaskItem = components['schemas']['TaskItem']

/** Which pile a task sits in: what it asks of the person first, then what it is doing. */
export type Pile = 'needs_you' | 'working' | 'finished'

export function pileOf(task: Pick<Task, 'status' | 'blocked'>): Pile {
  if (task.status === 'needs_you' || task.blocked) return 'needs_you'
  return task.status === 'working' ? 'working' : 'finished'
}

export const isRunning = (task: Task) => pileOf(task) !== 'finished'

export const titleOf = (task: Pick<Task, 'title' | 'request'>) =>
  task.title ?? (task.request.split('\n')[0] ?? '').slice(0, 80)

/** The task in a few words, for its row: what it waits on, or who is on what. */
export function sentenceOf(task: Task) {
  if (task.blocked) return 'An agent is stuck and waits for you'
  if (task.status === 'needs_you') return 'The orchestrator has a question'
  if (task.status === 'stopped') return 'Stopped'
  if (task.status === 'done') return 'Done'
  if (task.reading) return 'The orchestrator is reading'
  if (task.items.length === 0) return 'The orchestrator is planning'
  const current = task.items.find((item) => item.status === 'doing')
  const agent = task.agents.find((item) => item.id === current?.agent_id)
  return current && agent
    ? `${agent.name}: ${current.title}`
    : `${doneCount(task)} of ${task.items.length} done`
}

/** The card's line under the title; its plan already says who is on what. */
export function headlineOf(task: Task) {
  return pileOf(task) === 'working' && task.items.length > 0 && !task.reading
    ? 'Working'
    : sentenceOf(task)
}

/** A short stretch of the plan around what is happening now; the counts say what it leaves out. */
export function planWindow(items: TaskItem[], size = 5) {
  const doing = items.findIndex((item) => item.status === 'doing')
  const open = doing >= 0 ? doing : items.findIndex((i) => i.status === 'todo')
  const from = open >= 0 ? open - 1 : items.length
  const before = Math.max(0, Math.min(from, items.length - size))
  return {
    shown: items.slice(before, before + size),
    before,
    after: Math.max(0, items.length - before - size),
  }
}

export function doneCount(task: Pick<Task, 'items'>) {
  return task.items.filter(
    (item) => item.status === 'done' || item.status === 'skipped',
  ).length
}

/** Tasks in the pile, most recently active first as the server sends them. */
export function piles(tasks: Task[]) {
  const group = (pile: Pile) => tasks.filter((task) => pileOf(task) === pile)
  return {
    needs_you: group('needs_you'),
    working: group('working'),
    finished: group('finished'),
  }
}
