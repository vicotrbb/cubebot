import test from 'node:test'
import assert from 'node:assert/strict'
import { fitted, zoomed } from './view'

test('a chart larger than its frame shrinks to fit inside the padding', () => {
  const view = fitted({ w: 548, h: 348 }, 1000, 600)
  assert.equal(view.k, 0.5)
  assert.equal(view.x, 24)
  assert.equal(view.y, 24)
})

test('a small chart keeps its size and sits centred', () => {
  assert.deepEqual(fitted({ w: 800, h: 400 }, 400, 200), {
    k: 1,
    x: 200,
    y: 100,
  })
})

test('zooming keeps the point under the pointer still and respects the bounds', () => {
  const view = zoomed({ k: 1, x: 0, y: 0 }, 2, { x: 100, y: 50 }, 0.25)
  assert.deepEqual(view, { k: 2, x: -100, y: -50 })
  assert.equal(zoomed(view, 4, { x: 0, y: 0 }, 0.25).k, 2)
  assert.equal(zoomed(view, 0.01, { x: 0, y: 0 }, 0.25).k, 0.25)
})
