// @ts-expect-error bun provides this at test time
import { expect, test } from 'bun:test'
import { framed, framedSvg } from './framePage'

test('the policy comes before anything the page holds', () => {
  const page = framed(
    '<!DOCTYPE html><html><head><script>x()</script></head></html>',
  )
  expect(page.startsWith('<!doctype html>')).toBe(true)
  expect(page.match(/<!doctype/gi)?.length).toBe(1)
  expect(page.indexOf("default-src 'none'")).toBeLessThan(page.indexOf('x()'))
})

test('a drawing is centered inside the same policy', () => {
  const page = framedSvg('<svg></svg>')
  expect(page).toContain("default-src 'none'")
  expect(page).toContain('<svg></svg>')
})
