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
const sizes = [
  { name: 'wide', width: 1440, height: 900 },
  { name: 'phone', width: 390, height: 844 },
]

const browser = await chromium.launch()
try {
  const context = await browser.newContext({
    viewport: sizes[0],
    colorScheme: 'light',
  })
  const page = await context.newPage()
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
  await page.getByRole('link', { name: 'Apps', exact: true }).click()
  await page.waitForURL(/\/apps$/)

  for (const scheme of ['light', 'dark']) {
    for (const size of sizes) {
      await page.setViewportSize(size)
      await page.emulateMedia({ colorScheme: scheme })
      await page.goto(`${process.env.CUBEBOT_CONSOLE}/apps`)
      const card = page.locator('a.app-card', {
        has: page.locator('.app-card__name', { hasText: /^Status$/ }),
      })
      await card.waitFor()
      await card.getByText('Running').waitFor()
      await page.screenshot({
        path: join(evidence, `catalog-${size.name}-${scheme}.png`),
      })
      await card.click()
      const frame = page.locator('iframe[title="Status"]')
      await frame.waitFor()
      const sandbox = await frame.getAttribute('sandbox')
      assert.equal(
        sandbox,
        'allow-scripts allow-forms allow-popups allow-downloads',
      )
      assert.ok(!sandbox.includes('allow-same-origin'), sandbox)
      const shown = page
        .frameLocator('iframe[title="Status"]')
        .locator('#result')
      await shown
        .getByText(/^Live: .*authorization/)
        .waitFor({ timeout: 30000 })
      const theme = await page
        .frameLocator('iframe[title="Status"]')
        .locator('html')
        .getAttribute('data-theme')
      assert.equal(theme, scheme)
      await page.waitForTimeout(400)
      await page.screenshot({
        path: join(evidence, `app-${size.name}-${scheme}.png`),
      })
      console.log(`app rendered live data at ${size.name} in ${scheme}`)
    }
  }
  assert.deepEqual(failures, [])
} finally {
  await browser.close()
}
