import test from 'node:test'
import assert from 'node:assert/strict'
import { createServer } from 'node:http'
import { ApiError, agent, messageProblem, pages, rejectedInput } from './client'

test('message validation uses the server UTF-8 byte bound and keeps definite rejections editable', () => {
  assert.equal(messageProblem('a'.repeat(16384)), null)
  assert.match(messageProblem('界'.repeat(6000)) ?? '', /16 KiB/)
  assert.match(messageProblem('null\0byte') ?? '', /null/)
  assert.equal(rejectedInput(new ApiError(400)), true)
  assert.equal(rejectedInput(new ApiError(413)), true)
  assert.equal(rejectedInput(new ApiError(503)), false)
  assert.equal(rejectedInput(new Error('connection lost')), false)
})

test('agent loading follows every authorization-filtered page', async () => {
  const items = Array.from({ length: 101 }, (_, index) => ({
    id: String(index).padStart(3, '0'),
    name: 'Agent',
    instructions: '',
    revision: 1,
    idle_policy: 'sleep',
  }))
  const requests: string[] = []
  const server = createServer((request, response) => {
    const url = new URL(request.url ?? '/', 'http://localhost')
    const after = url.searchParams.get('after') ?? ''
    requests.push(after)
    response.setHeader('content-type', 'application/json')
    response.end(
      JSON.stringify(items.filter((item) => item.id > after).slice(0, 100)),
    )
  })
  await new Promise<void>((resolve) => server.listen(0, '127.0.0.1', resolve))
  const address = server.address()
  if (!address || typeof address === 'string')
    throw new Error('missing test listener')
  const original = globalThis.fetch
  globalThis.fetch = (input, init) =>
    original(
      typeof input === 'string'
        ? new URL(input, `http://127.0.0.1:${address.port}`)
        : input,
      init,
    )
  try {
    assert.equal((await pages('/agents', agent)).length, 101)
    assert.deepEqual(requests, ['', '099'])
  } finally {
    globalThis.fetch = original
    await new Promise<void>((resolve, reject) =>
      server.close((error) => (error ? reject(error) : resolve())),
    )
  }
})
