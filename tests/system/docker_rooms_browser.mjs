// Rooms, delegation and steering in real Chromium, against the agents docker_rooms.py has just run.
import assert from 'node:assert/strict'
import console from 'node:console'
import process from 'node:process'
import { createRequire } from 'node:module'
import { join } from 'node:path'

const require = createRequire(import.meta.url)
const { chromium } = require(
  process.env.CUBEBOT_PLAYWRIGHT_MODULE || 'playwright',
)
const evidence = process.env.CUBEBOT_EVIDENCE_DIRECTORY
const marker = process.env.CUBEBOT_ROOMS_MARK
const shot = (page, name) =>
  page.screenshot({ path: join(evidence, `${name}.png`), fullPage: true })

const browser = await chromium.launch()
try {
  const page = await browser.newPage({
    viewport: { width: 1440, height: 1000 },
  })
  const failures = []
  page.on('pageerror', (error) => failures.push(error.message))
  await page.goto(process.env.CUBEBOT_CONSOLE)
  await page
    .getByLabel('Login', { exact: true })
    .fill(process.env.CUBEBOT_CONSOLE_LOGIN)
  await page
    .getByLabel('Password', { exact: true })
    .fill(process.env.CUBEBOT_CONSOLE_PASSWORD)
  await page.getByRole('button', { name: 'Sign in', exact: true }).click()

  const rooms = page.getByRole('navigation', { name: 'Rooms' })
  await rooms.getByRole('link', { name: /^Launch\b/ }).click()
  await page.getByText(`ROUTE-${marker}`).first().waitFor()
  await shot(page, 'room')
  await page
    .getByRole('textbox', { name: 'Message the room' })
    .fill(`@Analyst reply with only the word BROWSER-${marker}`)
  await page
    .getByRole('button', { name: 'Message the room', exact: true })
    .click()
  await page
    .locator('.agent-message', { hasText: `BROWSER-${marker}` })
    .filter({ hasText: /Analyst\s*· replied/ })
    .waitFor({ timeout: 300_000 })
  await shot(page, 'room-streamed')
  await page.getByRole('link', { name: 'Edit members', exact: true }).click()
  for (const name of ['Lead', 'Analyst', 'Scribe'])
    assert.ok(await page.getByRole('checkbox', { name }).isChecked(), name)
  await shot(page, 'room-members')
  await page.getByRole('link', { name: 'Close members', exact: true }).click()

  const agents = page.getByRole('navigation', { name: 'Agents' })
  await agents.getByRole('link', { name: /^Lead\b/ }).click()
  await page.getByRole('button', { name: 'Delegated work' }).first().click()
  await page.getByRole('list', { name: 'Delegated work' }).waitFor()
  await page.getByText(`KIWI-${marker}`).first().waitFor()
  await page
    .getByText(/^Steering/)
    .first()
    .waitFor()
  await page.getByText('Reply from agent Scribe').first().waitFor()
  await shot(page, 'lead-chat')

  await agents.getByRole('link', { name: /^Analyst\b/ }).click()
  await page.getByText('Delegated by Lead').first().waitFor()
  await page.getByText('Room · Launch').first().waitFor()
  await page.getByText('Steering from the delegating work').first().waitFor()
  await shot(page, 'analyst-chat')
  await page.getByRole('link', { name: 'Details', exact: true }).click()
  await page
    .getByRole('navigation', { name: 'Agent details' })
    .getByRole('link', { name: 'About', exact: true })
    .click()
  const lanes = page.getByLabel('Delegated tasks at once')
  assert.equal(await lanes.inputValue(), '2')
  await shot(page, 'analyst-edit')

  // The sidebar is the Home page at phone width, so open it from the console root.
  await page.setViewportSize({ width: 390, height: 844 })
  await page.goto(process.env.CUBEBOT_CONSOLE)
  await rooms.getByRole('link', { name: /^Launch\b/ }).click()
  await page.getByText(`BROWSER-${marker}`).first().waitFor()
  const overflow = await page.evaluate(
    () =>
      globalThis.document.documentElement.scrollWidth > globalThis.innerWidth,
  )
  await shot(page, 'room-phone')
  assert.deepEqual(failures, [], 'page errors')
  assert.equal(overflow, false, 'horizontal scroll at phone width')
  console.log('console rooms, delegation and steering views verified')
} finally {
  await browser.close()
}
