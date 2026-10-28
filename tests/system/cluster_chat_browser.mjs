// Real Chromium with the disposable server's public key pinned, no blanket TLS bypass.
import assert from 'node:assert/strict'
import console from 'node:console'
import process from 'node:process'
import { createHash, X509Certificate } from 'node:crypto'
import { readFile, writeFile } from 'node:fs/promises'
import { createRequire } from 'node:module'
import { join } from 'node:path'

const require = createRequire(import.meta.url)
const { chromium } = require(
  process.env.CUBEBOT_PLAYWRIGHT_MODULE || 'playwright',
)
const directory = process.env.CUBEBOT_BROWSER_DIRECTORY
assert(directory)
const certificate = new X509Certificate(
  await readFile(join(directory, 'private/tls.crt')),
)
const pin = createHash('sha256')
  .update(certificate.publicKey.export({ type: 'spki', format: 'der' }))
  .digest('base64')
const browser = await chromium.launch({
  args: [`--ignore-certificate-errors-spki-list=${pin}`],
})
try {
  const page = await browser.newPage()
  await page.goto('https://127.0.0.1:33443')
  await page.getByLabel('Login', { exact: true }).fill('admin')
  await page
    .getByLabel('Password', { exact: true })
    .fill(process.env.CUBEBOT_BROWSER_PASSWORD)
  await page.getByRole('button', { name: 'Sign in', exact: true }).click()
  if (process.env.CUBEBOT_BROWSER_WAITING === '1') {
    await page.getByRole('button', { name: 'Provider unreachable' }).click()
    await page.getByText(/Waiting: provider unavailable/).waitFor()
    await page
      .getByRole('button', { name: 'Retry response', exact: true })
      .waitFor()
    await page.screenshot({
      path: join(directory, 'browser-waiting.png'),
      fullPage: true,
    })
    console.log('PASS browser exposes provider uncertainty and explicit retry')
  } else {
    await page.getByRole('button', { name: 'Live qualification' }).click()
    const input =
      'What word did I ask you to remember? Reply with that word only.'
    await page.getByLabel('Message', { exact: true }).fill(input)
    await page
      .getByRole('button', { name: 'Send message', exact: true })
      .click()
    await page.waitForFunction(
      () => {
        const replies = [
          ...globalThis.document.querySelectorAll('.agent-message p'),
        ]
        return (
          replies.length === 2 &&
          replies.every((reply) => !reply.hasAttribute('role'))
        )
      },
      null,
      { timeout: 100000 },
    )
    const replies = await page.locator('.agent-message p').allTextContents()
    assert.match(replies[1], /cedar/i)
    await page.reload()
    await page.getByRole('button', { name: 'Live qualification' }).click()
    await page.locator('.exchange').nth(1).waitFor()
    assert.deepEqual(
      await page.locator('.agent-message p').allTextContents(),
      replies,
    )
    const cookies = await page.context().cookies()
    const session = cookies.find((cookie) => cookie.name.startsWith('__Host-'))
    assert(session?.secure && session.httpOnly && session.sameSite === 'Lax')
    await page.screenshot({
      path: join(directory, 'browser-chat.png'),
      fullPage: true,
    })
    await writeFile(
      join(directory, 'browser.json'),
      JSON.stringify(
        {
          tlsPublicKeyPin: pin,
          secureCookie: true,
          historyAfterReload: true,
          credentialBackedContext: true,
          replies,
        },
        null,
        2,
      ),
    )
    console.log(
      'PASS browser sign-in, real runtime chat/context, Secure cookie and refresh',
    )
  }
} finally {
  await browser.close()
}
