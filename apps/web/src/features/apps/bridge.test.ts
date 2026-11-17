// @ts-expect-error bun provides this at test time
import { beforeEach, describe, expect, test } from 'bun:test'
import { bridge } from './bridge'

type Listener = (event: { source: unknown; data: unknown }) => void
type Sent = Record<string, unknown>

const globals = globalThis as never as Record<string, unknown>
let listeners: Listener[] = []
let observers: (() => void)[] = []
let dark = false
const root = { dataset: {} as Record<string, string> }

function fakeWindow() {
  const sent: Sent[] = []
  return {
    sent,
    window: { postMessage: (message: Sent) => sent.push(message) },
  }
}

beforeEach(() => {
  listeners = []
  observers = []
  dark = false
  root.dataset = {}
  globals.window = {
    addEventListener: (_: string, listener: Listener) =>
      listeners.push(listener),
    removeEventListener: (_: string, listener: Listener) => {
      listeners = listeners.filter((item) => item !== listener)
    },
  }
  globals.document = { documentElement: root }
  globals.matchMedia = () => ({
    get matches() {
      return dark
    },
    addEventListener: () => {},
    removeEventListener: () => {},
  })
  globals.MutationObserver = class {
    constructor(callback: () => void) {
      observers.push(callback)
    }
    observe() {}
    disconnect() {}
  }
})

const settle = () => new Promise((resolve) => setTimeout(resolve))
const deliver = (source: unknown, data: unknown) =>
  listeners.forEach((listener) => listener({ source, data }))

function mount(ask: (text: string) => Promise<void> = async () => {}) {
  const frame = fakeWindow()
  const asked: string[] = []
  const expiries: number[] = []
  const stop = bridge(
    { contentWindow: frame.window } as never as HTMLIFrameElement,
    {
      ask: (text) => {
        asked.push(text)
        return ask(text)
      },
      expired: () => expiries.push(1),
    },
  )
  return { frame, asked, expiries, stop }
}

describe('bridge', () => {
  test('messages_from_other_windows_are_ignored', async () => {
    const { frame, asked } = mount()
    const stranger = fakeWindow()
    deliver(stranger.window, { type: 'cubebot:ask', id: 1, text: 'hello' })
    deliver(stranger.window, { type: 'cubebot:ready' })
    await settle()
    expect(asked).toEqual([])
    expect(frame.sent).toEqual([])
    expect(stranger.sent).toEqual([])
  })

  test('ask_is_answered_once', async () => {
    const { frame, asked } = mount()
    deliver(frame.window, { type: 'cubebot:ask', id: 7, text: 'Plan my week' })
    deliver(frame.window, { type: 'cubebot:ask', id: 7, text: 'Plan my week' })
    await settle()
    expect(asked).toEqual(['Plan my week'])
    expect(frame.sent).toEqual([{ type: 'cubebot:asked', id: 7, ok: true }])
  })

  test('a page load restarts the ask ids', async () => {
    const { frame, asked } = mount()
    deliver(frame.window, { type: 'cubebot:ready' })
    deliver(frame.window, { type: 'cubebot:ask', id: 1, text: 'First' })
    await settle()
    deliver(frame.window, { type: 'cubebot:ready' })
    deliver(frame.window, { type: 'cubebot:ask', id: 1, text: 'Second' })
    await settle()
    expect(asked).toEqual(['First', 'Second'])
    expect(frame.sent.filter((item) => item.type === 'cubebot:asked')).toEqual([
      { type: 'cubebot:asked', id: 1, ok: true },
      { type: 'cubebot:asked', id: 1, ok: true },
    ])
  })

  test('an expired view is reported only from its own frame', () => {
    const { frame, expiries } = mount()
    deliver(fakeWindow().window, { type: 'cubebot:expired' })
    deliver(null, { type: 'cubebot:expired' })
    expect(expiries).toEqual([])
    deliver(frame.window, { type: 'cubebot:expired' })
    expect(expiries).toEqual([1])
    expect(frame.sent).toEqual([])
  })

  test('a failed ask is answered with its reason', async () => {
    const { frame } = mount(async () => {
      throw new Error('The agent is retired.')
    })
    deliver(frame.window, { type: 'cubebot:ask', id: 2, text: 'Hi' })
    await settle()
    expect(frame.sent).toEqual([
      {
        type: 'cubebot:asked',
        id: 2,
        ok: false,
        error: 'The agent is retired.',
      },
    ])
  })

  test('an ask with an unknown frame source is ignored', async () => {
    const { frame, asked } = mount()
    deliver(null, { type: 'cubebot:ask', id: 3, text: 'Hi' })
    deliver(undefined, { type: 'cubebot:ask', id: 4, text: 'Hi' })
    await settle()
    expect(asked).toEqual([])
    expect(frame.sent).toEqual([])
  })

  test('ready is answered with the current theme', () => {
    root.dataset.theme = 'dark'
    const { frame } = mount()
    deliver(frame.window, { type: 'cubebot:ready' })
    expect(frame.sent).toEqual([{ type: 'cubebot:theme', theme: 'dark' }])
  })

  test('a theme change is posted', () => {
    const { frame } = mount()
    dark = true
    observers.forEach((observe) => observe())
    root.dataset.theme = 'light'
    observers.forEach((observe) => observe())
    observers.forEach((observe) => observe())
    expect(frame.sent).toEqual([
      { type: 'cubebot:theme', theme: 'dark' },
      { type: 'cubebot:theme', theme: 'light' },
    ])
  })

  test('stopping detaches the listener', async () => {
    const { frame, asked, stop } = mount()
    stop()
    deliver(frame.window, { type: 'cubebot:ask', id: 5, text: 'Hi' })
    await settle()
    expect(asked).toEqual([])
  })
})
