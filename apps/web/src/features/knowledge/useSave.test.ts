// @ts-expect-error bun provides this at test time
import { afterEach, beforeEach, expect, mock, test } from 'bun:test'
import * as React from 'react'

class StaleSave extends Error {
  constructor(public revision: number) {
    super('stale')
  }
}
type Sent = { expected_revision: number; body?: string }
const saves: {
  sent: Sent
  resolve: (revision: number) => void
  reject: (error: unknown) => void
}[] = []
mock.module('./api', () => ({
  StaleSave,
  kbKeys: { page: (id: string) => ['kb', 'page', id] },
  savePage: (_: string, sent: Sent) =>
    new Promise((resolve, reject) => {
      saves.push({
        sent,
        resolve: (revision) => resolve({ revision }),
        reject,
      })
    }),
}))
const { useSave } = await import('./useSave')

const settle = () => new Promise((resolve) => setTimeout(resolve))
const internals = (React as never as Record<string, { H: unknown }>)
  .__CLIENT_INTERNALS_DO_NOT_USE_OR_WARN_USERS_THEY_CANNOT_UPGRADE as {
  H: unknown
}
const globals = globalThis as never as Record<string, unknown>
const realFetch = globals.fetch

let timers: (() => void)[] = []
let flushed: { expected_revision: number; body?: string }[] = []

/** No DOM to mount in here, so this stands in for React: one render, its effects, their cleanup. */
function mount() {
  const effects: (() => void | (() => void))[] = []
  internals.H = {
    useContext: () => ({ setQueryData: () => {} }),
    useState: (first: unknown) => [first, () => {}],
    useRef: (current: unknown) => ({ current }),
    useCallback: (callback: unknown) => callback,
    useEffect: (effect: () => void | (() => void)) => effects.push(effect),
  }
  const save = useSave({ id: 'p', revision: 1 } as never, () => {})
  internals.H = null
  const cleanups = effects.map((effect) => effect())
  return { save, unmount: () => cleanups.forEach((cleanup) => cleanup?.()) }
}

/** Types `body` and lets the quiet period pass. */
function typed(save: ReturnType<typeof useSave>, body: string) {
  save.change({ body })
  timers.splice(0).forEach((run) => run())
}

beforeEach(() => {
  saves.length = 0
  timers = []
  flushed = []
  globals.window = {
    setTimeout: (run: () => void) => timers.push(run),
    clearTimeout: () => {},
    addEventListener: () => {},
    removeEventListener: () => {},
  }
  globals.fetch = async (_: string, init: { body: string }) => {
    flushed.push(JSON.parse(init.body))
    throw new Error('offline')
  }
})

afterEach(() => {
  delete globals.window
  globals.fetch = realFetch
})

test('leaving during a save sends the rest after it, at the revision it returned', async () => {
  const { save, unmount } = mount()
  typed(save, 'first')
  save.change({ body: 'second' })
  unmount()
  // A second request now would carry the old revision and be refused, losing `second`.
  expect(flushed).toEqual([])
  expect(saves.length).toBe(1)
  saves[0]?.resolve(2)
  await settle()
  expect(saves[1]?.sent).toEqual({ expected_revision: 2, body: 'second' })
})

test('leaving with nothing under way sends what is waiting, and survives a failure', async () => {
  const { save, unmount } = mount()
  save.change({ body: 'unsaved' })
  unmount()
  await settle()
  expect(flushed).toEqual([{ expected_revision: 1, body: 'unsaved' }])
})

test('leaving a refused save sends nothing: it could only be refused again', async () => {
  const { save, unmount } = mount()
  typed(save, 'mine')
  saves[0]?.reject(new StaleSave(5))
  await settle()
  unmount()
  await settle()
  expect(flushed).toEqual([])
})
