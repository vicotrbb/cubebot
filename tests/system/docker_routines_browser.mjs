// Routines (ADR 0087) in real Chromium against the built console behind Caddy: the
// administrator makes a command routine with a webhook on Keeper's Routines tab, reads
// its secret once and its URL, runs it now, finds the run in its history and the turn
// labelled with the routine in the chat, and after a reload no secret is on the page.
// It prints SECRET= and HOOK= for the Python side to deliver with.
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
const marker = process.env.CUBEBOT_ROUTINES_MARKER
const name = `console-${marker}`

// The Routines tab lives in the agent's details panel.
async function routines(page) {
  await page
    .getByRole('navigation', { name: 'Agents' })
    .getByRole('link', { name: /^Keeper\b/ })
    .click()
  await page.getByRole('link', { name: 'Details', exact: true }).click()
  await page
    .getByRole('navigation', { name: 'Agent details' })
    .getByRole('link', { name: 'Routines', exact: true })
    .click()
  await page.getByRole('heading', { name: 'Routines', exact: true }).waitFor()
  return page.getByRole('complementary', { name: 'Details' })
}

const browser = await chromium.launch()
try {
  const page = await browser.newPage({
    viewport: { width: 1400, height: 1000 },
  })
  await page.goto(process.env.CUBEBOT_CONSOLE)
  await page
    .getByLabel('Login', { exact: true })
    .fill(process.env.CUBEBOT_CONSOLE_LOGIN)
  await page
    .getByLabel('Password', { exact: true })
    .fill(process.env.CUBEBOT_CONSOLE_PASSWORD)
  await page.getByRole('button', { name: 'Sign in', exact: true }).click()
  const panel = await routines(page)

  await page.getByRole('button', { name: 'New routine', exact: true }).click()
  await page.getByLabel('Name', { exact: true }).fill(name)
  // The label carries a hint ("Runs with bash before the prompt"), so no exact match.
  await page.getByLabel('Command').fill(`echo ${name}`)
  await page.getByLabel('Make a webhook').check()
  await page.screenshot({ path: join(evidence, 'routines-form.png') })
  await page.getByRole('button', { name: 'Save', exact: true }).click()

  const item = panel.locator('ul.list > li').filter({ hasText: name }).first()
  const shown = item.getByText('Secret, shown only now:')
  await shown.waitFor({ timeout: 15000 })
  const secret = (await shown.locator('code').textContent()).trim()
  assert.match(secret, /^[0-9a-f]{64}$/)
  const url = (
    await item.getByText('Webhook: POST').locator('code').first().textContent()
  ).trim()
  assert.ok(url.startsWith(`${process.env.CUBEBOT_CONSOLE}/api/hooks/`), url)

  await item.getByRole('button', { name: 'Run now', exact: true }).click()
  await page
    .getByText(`${name} is queued in the conversation.`)
    .waitFor({ timeout: 15000 })
  await item.locator('summary', { hasText: 'History' }).click()
  await item.getByText(/Run by a person · (queued|running|complete)/).waitFor()
  await page.screenshot({
    path: join(evidence, 'routines-page.png'),
    fullPage: true,
  })

  await page
    .getByRole('link', { name: 'Back to conversation', exact: true })
    .click()
  await page.getByText(`Routine · ${name}`).first().waitFor({ timeout: 15000 })
  // This run's answer, not an earlier routine's: the work card for this command reports
  // its exit status and output. The card collapses once the command ends, so open it.
  const work = page
    .locator('details.work')
    .filter({ hasText: `echo ${name}` })
    .last()
  await work
    .getByText('Exit status 0', { exact: true })
    .waitFor({ state: 'attached', timeout: 120000 })
  await work.evaluate((card) => {
    card.open = true
  })
  assert.match(
    await work.locator('pre.work__pre').last().textContent(),
    new RegExp(`^${name}\\s*$`),
  )
  await page.screenshot({ path: join(evidence, 'routines-chat.png') })

  await page.reload()
  const reopened = await routines(page)
  await reopened
    .locator('ul.list > li')
    .filter({ hasText: name })
    .first()
    .waitFor()
  assert.ok(!(await page.content()).includes(secret))
  assert.equal(await page.getByRole('alert').count(), 0)
  console.log(`SECRET=${secret}`)
  console.log(`HOOK=${url}`)
  console.log('console routines ok')
} finally {
  await browser.close()
}
