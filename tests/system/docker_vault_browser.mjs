// The vault (ADR 0086) in real Chromium against the built console behind Caddy: during a
// login handoff the administrator takes control from the chat's banner, saves the login
// to Keeper's vault with Save and fill, sees the fill picker and hands back, then finds
// the items on the Vault tab with no secret anywhere on it.
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
const site = process.env.CUBEBOT_VAULT_SITE
const secret = process.env.CUBEBOT_VAULT_SECRET
const secrets = JSON.parse(process.env.CUBEBOT_VAULT_SECRETS)
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
  await page
    .getByRole('navigation', { name: 'Agents' })
    .getByRole('link', { name: /^Keeper\b/ })
    .click()
  const banner = page.getByRole('region', { name: 'Login request' })
  await banner
    .getByText(`Keeper asks you to log in on ${site}.`)
    .waitFor({ timeout: 15000 })
  // Take control moves to the Desktop route, where the banner shows again.
  await banner
    .getByRole('button', { name: 'Take control', exact: true })
    .click()
  await page.getByText('You have control').waitFor({ timeout: 30000 })

  await banner.getByRole('button', { name: 'Save login', exact: true }).click()
  assert.equal(
    await banner.getByLabel('Name', { exact: true }).inputValue(),
    site,
  )
  assert.equal(
    await banner.getByLabel('Site', { exact: true }).inputValue(),
    site,
  )
  await banner.getByLabel('Username', { exact: true }).fill('ada')
  // The label carries a hint ("Never shown again"), so no exact match.
  await banner.getByLabel('Password').fill(secret)
  await page.screenshot({ path: join(evidence, 'vault-save-login.png') })
  await banner
    .getByRole('button', { name: 'Save and fill', exact: true })
    .click()
  await banner
    .getByRole('button', { name: 'Save login', exact: true })
    .waitFor({ timeout: 30000 })
  assert.equal(await page.getByRole('alert').count(), 0)

  // The picker loaded before the save; reopening the tab lists the new item too.
  const picker = page.getByLabel('Vault item to fill')
  await picker.waitFor()
  const options = await picker.locator('option').allTextContents()
  assert.ok(options.includes('Shared: password'), options)
  await page.screenshot({ path: join(evidence, 'vault-desktop.png') })
  await page.getByRole('button', { name: 'Hand back', exact: true }).click()
  await page.getByText('Watching the agent').waitFor()
  await banner.waitFor({ state: 'detached', timeout: 15000 })
  assert.equal(await picker.count(), 0)

  await page
    .getByRole('link', { name: 'Back to conversation', exact: true })
    .click()
  await page.getByRole('link', { name: 'Details', exact: true }).click()
  await page
    .getByRole('navigation', { name: 'Agent details' })
    .getByRole('link', { name: 'Vault', exact: true })
    .click()
  await page.getByRole('heading', { name: 'Vault', exact: true }).waitFor()
  for (const name of ['Shared', 'Keeper only', 'Own', 'Generated', site])
    await page.getByText(name, { exact: true }).first().waitFor()
  await page.getByRole('heading', { name: 'Installation items' }).waitFor()
  await page.getByText(`ada on ${site}`).first().waitFor()
  const html = await page.content()
  for (const value of [...secrets, secret]) assert.ok(!html.includes(value))
  assert.equal(await page.getByRole('alert').count(), 0)
  await page.screenshot({
    path: join(evidence, 'vault-page.png'),
    fullPage: true,
  })
  console.log('console vault ok')
} finally {
  await browser.close()
}
