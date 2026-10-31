// The login handoff banner (ADR 0039) in real Chromium against the built console behind
// Caddy. CUBEBOT_LOGIN_STEP is `take` (take control from the chat's banner and hand back
// on the Desktop tab) or `decline` (decline from the chat's banner with a reason).
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
const site = process.env.CUBEBOT_LOGIN_SITE
const step = process.env.CUBEBOT_LOGIN_STEP
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
  await page.getByRole('button', { name: /^Keeper\b/ }).click()
  const banner = page.getByRole('region', { name: 'Login request' })
  await banner
    .getByText(`Keeper asks you to log in on ${site}.`)
    .waitFor({ timeout: 15000 })
  await banner.getByText(`Console ${step}`).waitFor()
  await page.screenshot({ path: join(evidence, `login-${step}-chat.png`) })

  if (step === 'take') {
    await banner
      .getByRole('button', { name: 'Take control', exact: true })
      .click()
    await page.getByText('You have control').waitFor({ timeout: 30000 })
    await banner.getByText('then hand back control').waitFor()
    await page.screenshot({ path: join(evidence, 'login-take-desktop.png') })
    await page.getByRole('button', { name: 'Hand back', exact: true }).click()
    await page.getByText('Watching the agent').waitFor()
    await banner.waitFor({ state: 'detached', timeout: 15000 })
    await page
      .getByRole('button', { name: 'Back to conversation', exact: true })
      .click()
    await page
      .getByText(`Login completed on ${site}, take a screenshot.`)
      .waitFor({ timeout: 15000 })
  } else {
    await banner
      .getByLabel('Reason for declining')
      .fill('Not now from the console')
    await banner.getByRole('button', { name: 'Decline', exact: true }).click()
    await banner.waitFor({ state: 'detached', timeout: 15000 })
    await page
      .getByText(
        `A person declined to log in on ${site}: Not now from the console`,
      )
      .waitFor({ timeout: 15000 })
  }
  assert.equal(await page.getByRole('alert').count(), 0)
  await page.screenshot({
    path: join(evidence, `login-${step}-after.png`),
    fullPage: true,
  })
  console.log(`console ${step} ok`)
} finally {
  await browser.close()
}
