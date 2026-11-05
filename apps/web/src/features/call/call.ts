export type Line = {
  id: string
  who: 'you' | 'agent'
  text: string
  note?: string
}

export type CallState = {
  preparing: { model: string; share: number } | null
  ready: boolean
  partial: string
  lines: Line[]
  awaited: number
  notice: string
}

export const START: CallState = {
  preparing: null,
  ready: false,
  partial: '',
  lines: [],
  awaited: 0,
  notice: '',
}

export type Message =
  | { kind: 'preparing'; model: string; received: number; total: number }
  | { kind: 'ready' }
  | { kind: 'interrupt' }
  | { kind: 'partial'; text: string }
  | { kind: 'user'; turn: string; text: string }
  | { kind: 'agent'; turn: string; text: string }
  | { kind: 'tool'; turn: string; name: string }
  | { kind: 'queued'; turn: string }
  | { kind: 'answered'; turn: string; text: string | null }
  | { kind: 'unanswered'; turn: string; status: string; error: string | null }
  | { kind: 'notice'; text: string }

function withReply(lines: Line[], turn: string, change: (line: Line) => Line) {
  const id = `${turn}:agent`
  const known = lines.some((line) => line.id === id)
  const all = known
    ? lines
    : [...lines, { id, who: 'agent' as const, text: '' }]
  return all.map((line) => (line.id === id ? change(line) : line))
}

const unanswered = (status: string, error: string | null) => {
  if (status === 'cancelled') return 'Cancelled.'
  const reason = (error ?? 'unknown').replaceAll('_', ' ')
  return status === 'failed'
    ? `Failed: ${reason}.`
    : `Waiting: ${reason}. It is in the Call message after you hang up.`
}

export function reduce(state: CallState, message: Message): CallState {
  switch (message.kind) {
    case 'preparing':
      return {
        ...state,
        preparing: {
          model: message.model,
          share: message.total ? message.received / message.total : 0,
        },
      }
    case 'ready':
      return { ...state, ready: true, preparing: null }
    case 'partial':
      return { ...state, partial: message.text, notice: '' }
    case 'user':
      return {
        ...state,
        partial: '',
        awaited: state.awaited + 1,
        lines: [
          ...state.lines,
          { id: `${message.turn}:you`, who: 'you', text: message.text },
        ],
      }
    case 'agent':
      return {
        ...state,
        lines: withReply(state.lines, message.turn, (line) => ({
          ...line,
          text: line.text + message.text,
          note: '',
        })),
      }
    case 'tool':
      return {
        ...state,
        lines: withReply(state.lines, message.turn, (line) => ({
          ...line,
          note: `Using ${message.name.replaceAll('_', ' ')}…`,
        })),
      }
    case 'queued':
      return {
        ...state,
        lines: withReply(state.lines, message.turn, (line) => ({
          ...line,
          note: 'Queued until the agent is free.',
        })),
      }
    case 'answered':
      return {
        ...state,
        awaited: Math.max(0, state.awaited - 1),
        lines: withReply(state.lines, message.turn, (line) => ({
          ...line,
          text: message.text ?? line.text,
          note: '',
        })),
      }
    case 'unanswered':
      return {
        ...state,
        awaited: Math.max(0, state.awaited - 1),
        lines: withReply(state.lines, message.turn, (line) => ({
          ...line,
          note: unanswered(message.status, message.error),
        })),
      }
    case 'notice':
      return { ...state, notice: message.text }
    case 'interrupt':
      return state
  }
}

export type Phase = 'connecting' | 'listening' | 'thinking' | 'speaking'

export function phase(state: CallState, playing: boolean): Phase {
  if (!state.ready) return 'connecting'
  if (playing) return 'speaking'
  return state.awaited > 0 ? 'thinking' : 'listening'
}

/** A server audio frame: a little-endian sample rate, then PCM16. */
export function decodeAudio(frame: ArrayBuffer) {
  const rate = new DataView(frame).getUint32(0, true)
  const pcm = new Int16Array(frame.slice(4, 4 + ((frame.byteLength - 4) & ~1)))
  return { rate, samples: Float32Array.from(pcm, (value) => value / 32768) }
}

export function clock(seconds: number) {
  const minutes = Math.floor(seconds / 60)
  const rest = String(seconds % 60).padStart(2, '0')
  return `${minutes}:${rest}`
}

const NAMES = {
  embedded: 'Embedded',
  openai: 'OpenAI',
  elevenlabs: 'ElevenLabs',
} as const

export function providers(stt: keyof typeof NAMES, tts: keyof typeof NAMES) {
  return stt === tts ? NAMES[stt] : `${NAMES[stt]} and ${NAMES[tts]}`
}

export const PROVIDER_OPTIONS = Object.entries(NAMES) as [
  keyof typeof NAMES,
  string,
][]
