import test from 'node:test'
import assert from 'node:assert/strict'
import {
  clock,
  decodeAudio,
  phase,
  providers,
  reduce,
  START,
  type Message,
} from './call'

const play = (messages: Message[]) => messages.reduce(reduce, START)

test('a turn runs from the partial through the reply', () => {
  const heard = play([{ kind: 'ready' }, { kind: 'partial', text: 'What is' }])
  assert.equal(heard.partial, 'What is')
  assert.equal(phase(heard, false), 'listening')
  const asked = reduce(heard, { kind: 'user', turn: 't', text: 'What is it' })
  assert.equal(asked.partial, '')
  assert.equal(phase(asked, false), 'thinking')
  const replying = reduce(asked, { kind: 'agent', turn: 't', text: 'A cube' })
  assert.equal(phase(replying, true), 'speaking')
  const done = reduce(replying, {
    kind: 'answered',
    turn: 't',
    text: 'A cube.',
  })
  assert.equal(done.lines.at(-1)?.text, 'A cube.')
  assert.equal(done.awaited, 0)
  assert.deepEqual(
    done.lines.map((line) => line.who),
    ['you', 'agent'],
  )
})

test('a reply that cannot finish says why and stops waiting', () => {
  const state = play([
    { kind: 'ready' },
    { kind: 'user', turn: 't', text: 'Hi' },
    { kind: 'unanswered', turn: 't', status: 'failed', error: 'model_refused' },
  ])
  assert.equal(state.lines.at(-1)?.note, 'Failed: model refused.')
  assert.equal(phase(state, false), 'listening')
})

test('a tool shows until the reply resumes', () => {
  const working = play([
    { kind: 'user', turn: 't', text: 'Hi' },
    { kind: 'tool', turn: 't', name: 'kb_search' },
  ])
  assert.equal(working.lines.at(-1)?.note, 'Using kb search…')
  const reply = reduce(working, { kind: 'agent', turn: 't', text: 'Found' })
  assert.equal(reply.lines.at(-1)?.note, '')
})

test('a server frame carries its rate before the samples', () => {
  const bytes = new Uint8Array([0x22, 0x56, 0, 0, 0, 0x40, 0, 0xc0])
  const { rate, samples } = decodeAudio(bytes.buffer)
  assert.equal(rate, 22050)
  assert.deepEqual([...samples], [0.5, -0.5])
})

test('labels', () => {
  assert.equal(clock(65), '1:05')
  assert.equal(providers('embedded', 'embedded'), 'Embedded')
  assert.equal(providers('openai', 'elevenlabs'), 'OpenAI and ElevenLabs')
})
