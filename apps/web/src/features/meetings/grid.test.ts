import test from 'node:test'
import assert from 'node:assert/strict'
import { bestGrid } from './grid'

test('one person fills a wide stage by height', () => {
  assert.deepEqual(bestGrid(1, 1600, 720, 8), {
    columns: 1,
    width: 1280,
    height: 720,
  })
})

test('two people sit side by side on a wide screen', () => {
  assert.equal(bestGrid(2, 1600, 800, 8).columns, 2)
})

test('two people stack on a tall phone', () => {
  assert.equal(bestGrid(2, 390, 700, 8).columns, 1)
})

test('nine people make a three by three grid on a desktop', () => {
  assert.equal(bestGrid(9, 1440, 810, 8).columns, 3)
})

test('an empty room still answers a usable grid', () => {
  assert.equal(bestGrid(0, 800, 600, 8).columns, 1)
})

test('a portrait phone gives one person a tall tile', () => {
  assert.deepEqual(bestGrid(1, 360, 640, 8, 3 / 4), {
    columns: 1,
    width: 360,
    height: 480,
  })
})
