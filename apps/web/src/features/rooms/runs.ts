import type { RoomMessage } from '../../api/client'

type Delivery = RoomMessage['deliveries'][number]

export type Part = { handoff: RoomMessage; delivery: Delivery }

export type Runs = {
  top: RoomMessage[]
  parts: (turn: string) => Part[]
  notes: (turn: string) => RoomMessage[]
}

export type Tally = {
  total: number
  done: number
  working: number
  failed: number
  waiting: number
}

const WORKING = ['queued', 'running', 'delegated']
const FAILED = ['failed', 'cancelled']

function push<T>(map: Map<string, T[]>, key: string, item: T) {
  map.set(key, [...(map.get(key) ?? []), item])
}

export function runs(messages: RoomMessage[]): Runs {
  const turns = new Set(
    messages.flatMap((message) => message.deliveries.map((item) => item.turn)),
  )
  const parts = new Map<string, Part[]>()
  const notes = new Map<string, RoomMessage[]>()
  const nested = new Set<string>()
  for (const message of messages.filter((item) => item.author_agent_id)) {
    for (const delivery of message.deliveries) {
      const parent = delivery.parent_turn
      if (!parent || !turns.has(parent)) continue
      push(parts, parent, { handoff: message, delivery })
      nested.add(message.id)
    }
    if (message.steered && turns.has(message.steered)) {
      push(notes, message.steered, message)
      nested.add(message.id)
    }
  }
  return {
    top: messages.filter((message) => !nested.has(message.id)),
    parts: (turn) => parts.get(turn) ?? [],
    notes: (turn) => notes.get(turn) ?? [],
  }
}

export function tally(run: Runs, turn: string): Tally {
  const counted: Tally = {
    total: 0,
    done: 0,
    working: 0,
    failed: 0,
    waiting: 0,
  }
  const visit = (parent: string) => {
    for (const { delivery } of run.parts(parent)) {
      counted.total += 1
      if (delivery.status === 'complete') counted.done += 1
      else if (WORKING.includes(delivery.status)) counted.working += 1
      else if (FAILED.includes(delivery.status)) counted.failed += 1
      else counted.waiting += 1
      visit(delivery.turn)
    }
  }
  visit(turn)
  return counted
}

export function phrase(counted: Tally) {
  const words = [
    `${counted.total} ${counted.total === 1 ? 'part' : 'parts'}`,
    counted.done && `${counted.done} done`,
    counted.working && `${counted.working} working`,
    counted.waiting && `${counted.waiting} waiting`,
    counted.failed && `${counted.failed} failed`,
  ]
  return words.filter(Boolean).join(' · ')
}
