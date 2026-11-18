import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { join } from 'node:path'
import test from 'node:test'
import { runInNewContext } from 'node:vm'

const source = readFileSync(
  join(import.meta.dirname, '../../../../../modules/kit/src/kit.js'),
  'utf8',
)

function framedKit(answer: Response) {
  const posted: unknown[] = []
  const parent = { postMessage: (message: unknown) => posted.push(message) }
  const window = { parent, addEventListener() {} } as Record<string, unknown>
  runInNewContext(source, {
    window,
    document: {
      currentScript: { src: 'https://cubebot.lan/v/t/_cubebot/kit.js' },
      documentElement: { dataset: {} },
    },
    location: { search: '' },
    URL,
    URLSearchParams,
    Map,
    fetch: () => Promise.resolve(answer),
  })
  const kit = window.cubebot as {
    call: (tool: string) => Promise<string>
  }
  return { kit, posted }
}

function json(status: number, body: object) {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  })
}

test('a refused call asks the console for a fresh view', async () => {
  const { kit, posted } = framedKit(json(404, { error: 'not_found' }))
  await assert.rejects(kit.call('whoop__list_cycles'), /expired/)
  assert.equal((posted.at(-1) as { type: string }).type, 'cubebot:expired')
})

test('a failing tool keeps its own error and the view', async () => {
  const { kit, posted } = framedKit(json(403, { error: 'tool not declared' }))
  await assert.rejects(kit.call('github__delete'), /tool not declared/)
  assert.ok(
    !posted.some(
      (item) => (item as { type: string }).type === 'cubebot:expired',
    ),
  )
})
