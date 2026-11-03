// A fresh installation's console offers the setup form, and creating the administrator
// there lands in the console. CUBEBOT_CONSOLE is the installation's origin.
import console from 'node:console'
import process from 'node:process'
import { createRequire } from 'node:module'

const require = createRequire(import.meta.url)
const { chromium } = require(
  process.env.CUBEBOT_PLAYWRIGHT_MODULE || 'playwright',
)
const browser = await chromium.launch()
try {
  const page = await browser.newPage()
  await page.goto(process.env.CUBEBOT_CONSOLE)
  await page
    .getByText(
      'This installation has no accounts yet. Create the administrator.',
    )
    .waitFor({ timeout: 30000 })
  await page.getByLabel('Login', { exact: true }).fill('admin')
  const password = 'install setup browser password'
  await page.getByLabel('Password', { exact: true }).fill(password)
  await page.getByLabel('Confirm password', { exact: true }).fill(password)
  await page
    .getByRole('button', { name: 'Create administrator', exact: true })
    .click()
  await page
    .getByRole('navigation', { name: 'Workspace' })
    .waitFor({ timeout: 30000 })
  console.log('setup form ok')
} finally {
  await browser.close()
}
