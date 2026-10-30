// The console's Desktop tab in real Chromium against the built web server behind Caddy.
import assert from 'node:assert/strict'
import console from 'node:console'
import process from 'node:process'
import { createRequire } from 'node:module'
import { join } from 'node:path'
import { setTimeout as sleep } from 'node:timers/promises'

const require = createRequire(import.meta.url)
const { chromium } = require(
  process.env.CUBEBOT_PLAYWRIGHT_MODULE || 'playwright',
)
const evidence = process.env.CUBEBOT_EVIDENCE_DIRECTORY
const browser = await chromium.launch()
try {
  const page = await browser.newPage({
    viewport: { width: 1600, height: 1200 },
  })
  await page.goto(process.env.CUBEBOT_CONSOLE)
  await page
    .getByLabel('Login', { exact: true })
    .fill(process.env.CUBEBOT_CONSOLE_LOGIN)
  await page
    .getByLabel('Password', { exact: true })
    .fill(process.env.CUBEBOT_CONSOLE_PASSWORD)
  await page.getByRole('button', { name: 'Sign in', exact: true }).click()
  await page.getByRole('button', { name: /^Desk\b/ }).click()
  await page.getByRole('button', { name: 'Desktop', exact: true }).click()
  await page.getByRole('button', { name: 'Watch', exact: true }).click()
  await page.getByText('Watching the agent').waitFor()
  const canvas = page.locator('.desktop-screen canvas')
  // The guest's screen is painted: the canvas is no longer uniformly blank.
  await page.waitForFunction(() => {
    const element = globalThis.document.querySelector('.desktop-screen canvas')
    const pixels = element
      .getContext('2d')
      .getImageData(0, 0, element.width, element.height).data
    const seen = new Set()
    for (let at = 0; at < pixels.length; at += 4 * 997)
      seen.add(`${pixels[at]},${pixels[at + 1]},${pixels[at + 2]}`)
    return seen.size > 8
  })
  await page.locator('.desktop-pointer').waitFor()
  await page.screenshot({ path: join(evidence, 'desktop-watch.png') })

  await page.getByRole('button', { name: 'Take control', exact: true }).click()
  await page.getByText('You have control').waitFor()
  await page.getByRole('button', { name: 'Hand back', exact: true }).waitFor()
  const box = await canvas.boundingBox()
  // The guest's address bar, in its 1280x800 screen.
  await page.mouse.click(
    box.x + (600 / 1280) * box.width,
    box.y + (63 / 800) * box.height,
  )
  await page.keyboard.press('Control+l')
  await page.keyboard.type(process.env.CUBEBOT_PROOF_URL, { delay: 20 })
  await page.keyboard.press('Enter')
  await sleep(6000)
  await page.mouse.click(box.x + box.width / 2, box.y + box.height / 2)
  await page.locator('.desktop-pointer.you').waitFor()
  await sleep(3000)
  const latency = await page.locator('.desktop-latency').textContent()
  assert.match(latency, /Input round trip: \d+ ms \(median \d+ ms of \d+\)/)
  await page.screenshot({ path: join(evidence, 'desktop-control.png') })

  await page.getByRole('button', { name: 'Hand back', exact: true }).click()
  await page.getByText('Watching the agent').waitFor()
  await page
    .getByRole('button', { name: 'Back to conversation', exact: true })
    .click()
  await page.locator('.desktop-marker').first().waitFor()
  await page.screenshot({
    path: join(evidence, 'desktop-transcript.png'),
    fullPage: true,
  })
  const [, last, median, count] = latency.match(
    /(\d+) ms \(median (\d+) ms of (\d+)\)/,
  )
  const updates = Number(latency.match(/Screen updates: (\d+)/)[1])
  console.log(
    JSON.stringify({
      input_round_trip_last_ms: Number(last),
      input_round_trip_median_ms: Number(median),
      samples: Number(count),
      screen_updates_per_second: updates,
    }),
  )
} finally {
  await browser.close()
}
