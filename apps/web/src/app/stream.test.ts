// @ts-expect-error bun provides this at test time
import { afterEach, describe, expect, test } from 'bun:test'
import { followWhileVisible } from './stream'

const CLOSED = 2

class FakeSource {
  static CLOSED = CLOSED
  static made: FakeSource[] = []
  readyState = 0
  closed = false
  listeners: (() => void)[] = []
  constructor(public url: string) {
    FakeSource.made.push(this)
  }
  addEventListener(_: string, listener: () => void) {
    this.listeners.push(listener)
  }
  close() {
    this.closed = true
  }
  fail(state: number) {
    this.readyState = state
    for (const listener of this.listeners) listener()
  }
}

const globals = globalThis as never as Record<string, unknown>
const real = { setTimeout, clearTimeout }

function page() {
  const state = { hidden: false, onChange: () => {} }
  const timers = new Map<number, () => void>()
  let next = 0
  FakeSource.made = []
  globals.EventSource = FakeSource
  globals.document = {
    get hidden() {
      return state.hidden
    },
    addEventListener: (_: string, listener: () => void) => {
      state.onChange = listener
    },
    removeEventListener: () => {},
  }
  globals.setTimeout = (run: () => void) => {
    timers.set(++next, run)
    return next
  }
  globals.clearTimeout = (id: number) => timers.delete(id)
  const fire = () => {
    const due = [...timers.values()]
    timers.clear()
    for (const run of due) run()
  }
  return { state, timers, fire }
}

afterEach(() => {
  Object.assign(globalThis, real)
  delete globals.document
  delete globals.EventSource
})

describe('followWhileVisible', () => {
  test('replaces a source the browser gave up on, with setup run again', () => {
    const { timers, fire } = page()
    let setups = 0
    const stop = followWhileVisible('/x', () => setups++)
    FakeSource.made[0]?.fail(CLOSED)
    expect(FakeSource.made.length).toBe(1)
    fire()
    expect(FakeSource.made.length).toBe(2)
    expect(setups).toBe(2)
    expect(timers.size).toBe(0)
    stop()
  })
  test('leaves the reconnecting the browser does itself alone', () => {
    const { timers } = page()
    const stop = followWhileVisible('/x', () => {})
    FakeSource.made[0]?.fail(0)
    expect(timers.size).toBe(0)
    stop()
  })
  test('a hidden page waits, and returning opens one source only', () => {
    const { state, fire } = page()
    const stop = followWhileVisible('/x', () => {})
    FakeSource.made[0]?.fail(CLOSED)
    state.hidden = true
    fire()
    expect(FakeSource.made.length).toBe(1)
    state.hidden = false
    state.onChange()
    expect(FakeSource.made.length).toBe(2)
    fire()
    expect(FakeSource.made.length).toBe(2)
    stop()
  })
  test('stopping cancels a waiting retry', () => {
    const { timers } = page()
    const stop = followWhileVisible('/x', () => {})
    FakeSource.made[0]?.fail(CLOSED)
    stop()
    expect(timers.size).toBe(0)
  })
  test('a url given as a function is read for each new source', () => {
    const { fire } = page()
    let cursor = 0
    const stop = followWhileVisible(
      () => `/x?after=${cursor}`,
      () => {},
    )
    cursor = 7
    FakeSource.made[0]?.fail(CLOSED)
    fire()
    expect(FakeSource.made.map((source) => source.url)).toEqual([
      '/x?after=0',
      '/x?after=7',
    ])
    stop()
  })
})
