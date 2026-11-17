import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import test from 'node:test'
import { buildKit, kitPath } from '../scripts/kit'

test('the committed kit matches the console styles', () => {
  assert.equal(readFileSync(kitPath, 'utf8'), buildKit())
})

test('the kit carries no font files, comments or card-less console chrome', () => {
  const css = buildKit()
  assert.ok(!css.includes('@font-face'))
  assert.ok(!css.includes('/*'))
  assert.ok(css.includes('.btn'))
  assert.ok(css.includes(":root[data-theme='dark']"))
  assert.ok(css.includes('.kit-card'))
})
