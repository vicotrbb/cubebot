import test from 'node:test'
import assert from 'node:assert/strict'
import { dropTarget, pathTo, type Summary, under } from './tree'

const page = (id: string, parent: string | null = null) =>
  ({ id, parent, kind: 'page', title: id }) as Summary
const pages = [
  page('a'),
  page('b'),
  page('c', 'b'),
  page('d', 'b'),
  page('e', 'c'),
]
const find = (id: string) => pages.find((item) => item.id === id) as Summary

test('a path runs from the top down to the page', () => {
  assert.deepEqual(
    pathTo(pages, 'e').map((item) => item.id),
    ['b', 'c', 'e'],
  )
  assert.deepEqual(pathTo(pages, 'nothing'), [])
})

test('under finds every descendant', () => {
  assert.deepEqual([...under(pages, 'b')].sort(), ['c', 'd', 'e'])
})

test('a drop names the parent and the sibling it follows', () => {
  assert.deepEqual(dropTarget(pages, 'a', find('c'), 'after'), {
    parent: 'b',
    after: 'c',
  })
  assert.deepEqual(dropTarget(pages, 'a', find('c'), 'before'), {
    parent: 'b',
    after: null,
  })
  assert.deepEqual(dropTarget(pages, 'a', find('d'), 'before'), {
    parent: 'b',
    after: 'c',
  })
  assert.deepEqual(dropTarget(pages, 'a', find('b'), 'inside'), {
    parent: 'b',
    after: 'd',
  })
})

test('a page cannot drop onto itself or into its own subtree', () => {
  assert.equal(dropTarget(pages, 'b', find('b'), 'inside'), null)
  assert.equal(dropTarget(pages, 'b', find('e'), 'after'), null)
})
