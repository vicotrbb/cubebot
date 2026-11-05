// A call in real Chromium: a human recording in place of the microphone device, the real speech stack
// behind a small reverse proxy that stands where an ingress does.
import assert from 'node:assert/strict'
import console from 'node:console'
import { readFileSync } from 'node:fs'
import http from 'node:http'
import https from 'node:https'
import { createRequire } from 'node:module'
import { join } from 'node:path'
import process from 'node:process'

const require = createRequire(import.meta.url)
const { chromium, firefox } = require(
  process.env.CUBEBOT_PLAYWRIGHT_MODULE || 'playwright',
)
const env = process.env
const orchestrator = new globalThis.URL(env.CUBEBOT_ORCHESTRATOR)
const ca = readFileSync(env.CUBEBOT_CA)
const evidence = env.CUBEBOT_EVIDENCE_DIRECTORY

function upstream(url, method, headers) {
  const forwarded = { method, path: url, headers }
  if (url.startsWith('/api/'))
    return https.request({
      ...forwarded,
      host: orchestrator.hostname,
      port: orchestrator.port,
      ca,
    })
  return http.request({
    ...forwarded,
    host: '127.0.0.1',
    port: env.CUBEBOT_WEB_PORT,
  })
}

const proxy = http.createServer((request, response) => {
  const out = upstream(request.url, request.method, request.headers)
  out.on('response', (answer) => {
    response.writeHead(answer.statusCode, answer.headers)
    answer.pipe(response)
  })
  out.on('error', () => response.destroy())
  request.pipe(out)
})
proxy.on('upgrade', (request, socket, head) => {
  const out = upstream(request.url, request.method, request.headers)
  out.on('upgrade', (answer, upstreamSocket, upstreamHead) => {
    const lines = Object.entries(answer.headers).map(
      ([name, value]) => `${name}: ${value}`,
    )
    socket.write(
      `HTTP/1.1 101 Switching Protocols\r\n${lines.join('\r\n')}\r\n\r\n`,
    )
    if (upstreamHead.length) socket.write(upstreamHead)
    if (head.length) upstreamSocket.write(head)
    upstreamSocket.pipe(socket).pipe(upstreamSocket)
    socket.on('error', () => upstreamSocket.destroy())
    upstreamSocket.on('error', () => socket.destroy())
  })
  out.on('response', (answer) => {
    socket.end(`HTTP/1.1 ${answer.statusCode} Refused\r\n\r\n`)
  })
  out.on('error', () => socket.destroy())
  out.end()
})
await new Promise((resolve) =>
  proxy.listen(Number(env.CUBEBOT_PROXY_PORT), '127.0.0.1', resolve),
)

const browser = await chromium.launch()
const recording = readFileSync(env.CUBEBOT_MICROPHONE).toString('base64')
// Stands where the device would: everything after getUserMedia is the production path. Chromium's
// fake capture flags hang getUserMedia on the Mac this ran on, so they cannot be relied on.
function speakIntoMicrophone(base64) {
  globalThis.navigator.mediaDevices.getUserMedia = async () => {
    const context = new globalThis.AudioContext()
    const bytes = Uint8Array.from(globalThis.atob(base64), (at) =>
      at.charCodeAt(0),
    )
    const source = context.createBufferSource()
    source.buffer = await context.decodeAudioData(bytes.buffer)
    source.loop = true
    const sink = context.createMediaStreamDestination()
    source.connect(sink)
    source.start()
    return sink.stream
  }
}
const failures = []
const pages = []
try {
  const context = await browser.newContext({
    viewport: { width: 1280, height: 860 },
    colorScheme: 'light',
    serviceWorkers: 'block',
  })
  await context.addInitScript(speakIntoMicrophone, recording)
  const page = await context.newPage()
  page.on('pageerror', (error) => failures.push(String(error)))
  page.on('console', (message) => console.log('page:', message.text()))
  pages.push(page)
  await page.goto(`http://127.0.0.1:${env.CUBEBOT_PROXY_PORT}/`)
  await page
    .getByLabel('Login', { exact: true })
    .fill(env.CUBEBOT_CONSOLE_LOGIN)
  await page
    .getByLabel('Password', { exact: true })
    .fill(env.CUBEBOT_CONSOLE_PASSWORD)
  await page.getByRole('button', { name: 'Sign in', exact: true }).click()
  await page
    .getByRole('navigation', { name: 'Agents' })
    .getByRole('link', { name: /^Caller\b/ })
    .click()

  // A voice far down the long Details panel: choosing it must not scroll the shell out of view.
  await page.getByRole('link', { name: 'Details' }).click()
  const voice = page.getByRole('radio', { name: 'Male', exact: true })
  await voice.locator('..').click()
  assert.equal(await voice.isChecked(), true)
  assert.equal(
    await page.locator('.shell').evaluate((shell) => shell.scrollTop),
    0,
  )
  await page.goBack()
  console.log('PASS choosing a voice in Details keeps the page in view')

  // Call sits immediately before Desktop and Details.
  const actions = await page
    .locator('.agent-head__actions > :is(a, button)')
    .evaluateAll((items) =>
      items.map((item) => item.getAttribute('aria-label')),
    )
  const at = actions.indexOf('Call')
  assert.equal(actions[at + 1], 'Desktop', actions.join())
  assert.equal(actions[at + 2], 'Details', actions.join())

  await page.getByRole('button', { name: 'Call', exact: true }).click()
  const screen = page.getByRole('dialog', { name: 'Call with Caller' })
  await screen.waitFor()
  await page.evaluate(() => {
    const seen = (globalThis.phases = [])
    const element = globalThis.document.querySelector('.call-screen')
    const note = () => {
      const at = globalThis.performance.now()
      const phase = element.dataset.phase
      if (seen.at(-1)?.phase !== phase) seen.push({ phase, at })
    }
    note()
    new globalThis.MutationObserver(note).observe(element, { attributes: true })
  })

  // The recording reaches Zipformer through the worklet, partials first.
  await screen
    .getByText(/yellow|early|night/i)
    .first()
    .waitFor({ timeout: 60000 })
  console.log('PASS a partial transcript arrived while the person spoke')
  await screen
    .locator('.call-line--you')
    .getByText(/brothels/i)
    .waitFor({ timeout: 60000 })
  console.log('PASS Silero ended the sentence and it became a turn')
  await screen.getByText('Hello from the cube').waitFor({ timeout: 60000 })
  await page.waitForFunction(
    () => globalThis.phases.some((seen) => seen.phase === 'speaking'),
    undefined,
    { timeout: 30000 },
  )
  console.log('PASS the reply streamed back and Piper audio played')
  const firstAudio = await page.evaluate(() => {
    const at = globalThis.phases.findIndex((seen) => seen.phase === 'speaking')
    return globalThis.phases[at].at - globalThis.phases[at - 1].at
  })
  console.log(
    `MEASURE sentence recorded to first audio: ${Math.round(firstAudio)} ms`,
  )
  await page.screenshot({ path: join(evidence, 'call-desktop-light.png') })

  // The recording loops: the second question gets a reply of many sentences, and the third
  // saying of it starts while that reply is playing.
  await screen.getByText(/sentence number 3 of/).waitFor({ timeout: 90000 })
  const spokeAt = await page.waitForFunction(
    () => {
      const last = globalThis.phases.at(-1)
      return last.phase === 'speaking' ? last.at : null
    },
    undefined,
    { timeout: 60000 },
  )
  const speaking = await spokeAt.jsonValue()
  await page.waitForFunction(
    (since) =>
      globalThis.phases.some(
        (seen) => seen.at > since && seen.phase !== 'speaking',
      ),
    speaking,
    { timeout: 45000, polling: 100 },
  )
  // Sixty sentences last minutes; stopping this soon can only be the interruption.
  const stoppedAfter =
    (await page.evaluate(() => globalThis.phases.at(-1).at)) - speaking
  assert.ok(stoppedAfter < 45000, `playback ran ${stoppedAfter} ms`)
  console.log(
    `PASS speaking stopped playback after ${Math.round(stoppedAfter)} ms`,
  )
  await page.screenshot({ path: join(evidence, 'call-speaking.png') })

  await screen.getByRole('button', { name: 'Hang up' }).click()
  await screen.waitFor({ state: 'detached' })

  // One Call message, however many turns the call took.
  const message = page.locator('.call-message')
  await message.first().waitFor({ timeout: 15000 })
  await page.waitForFunction(
    () =>
      !globalThis.document
        .querySelector('.call-message__summary')
        ?.textContent.includes('On the call'),
    undefined,
    { timeout: 15000 },
  )
  assert.equal(await message.count(), 1)
  const summary = await message.locator('.call-message__summary').textContent()
  assert.match(summary, /^\d+:\d\d · Embedded · \d+ turns/, summary)
  await page
    .locator('.transcript .exchange')
    .first()
    .waitFor({ state: 'detached' })
    .catch(() => {})
  assert.equal(
    await page.locator('.transcript > .exchange').count(),
    0,
    'no turn of the call shows as its own message',
  )
  await page.screenshot({ path: join(evidence, 'call-message-closed.png') })
  await message.getByRole('button').first().click()
  const transcript = message.locator('.call-message__transcript')
  await transcript
    .getByText(/yellow lamps/i)
    .first()
    .waitFor()
  await transcript.getByText('Hello from the cube').waitFor()
  await page.screenshot({ path: join(evidence, 'call-message-open.png') })
  console.log(`PASS one Call message: ${summary}`)

  await page.reload()
  await message.first().waitFor({ timeout: 15000 })
  assert.equal(await message.count(), 1)
  console.log('PASS the message survives a reload')

  await page.emulateMedia({ colorScheme: 'dark' })
  await page.screenshot({ path: join(evidence, 'call-message-dark.png') })

  const phone = await browser.newContext({
    viewport: { width: 390, height: 844 },
    deviceScaleFactor: 2,
    hasTouch: true,
    isMobile: true,
    colorScheme: 'dark',
    serviceWorkers: 'block',
    storageState: await context.storageState(),
  })
  await phone.addInitScript(speakIntoMicrophone, recording)
  const small = await phone.newPage()
  await small.goto(page.url())
  const opener = small.locator('.call-message').getByRole('button').first()
  await opener.waitFor({ timeout: 15000 })
  await small.screenshot({
    path: join(evidence, 'call-message-phone-dark.png'),
  })
  await small.getByRole('button', { name: 'Call', exact: true }).click()
  const phoneScreen = small.getByRole('dialog', { name: 'Call with Caller' })
  await phoneScreen
    .locator('.call-line--you')
    .first()
    .waitFor({ timeout: 60000 })
  await small.screenshot({ path: join(evidence, 'call-phone-dark.png') })
  const hangup = await phoneScreen
    .getByRole('button', { name: 'Hang up' })
    .boundingBox()
  assert.ok(hangup.y + hangup.height <= 844, 'hang up is on screen')
  assert.equal(
    await small.evaluate(
      () =>
        globalThis.document.documentElement.scrollWidth <=
        globalThis.innerWidth,
    ),
    true,
    'no sideways scroll',
  )
  console.log('PASS a call on a phone viewport, dark')
  assert.deepEqual(failures, [])
  if (env.CUBEBOT_FIREFOX) {
    const other = await firefox.launch({ executablePath: env.CUBEBOT_FIREFOX })
    const visit = await other.newContext({
      serviceWorkers: 'block',
      storageState: await context.storageState(),
    })
    await visit.addInitScript(speakIntoMicrophone, recording)
    const fox = await visit.newPage()
    await fox.goto(page.url())
    await fox.getByRole('button', { name: 'Call', exact: true }).click()
    const call = fox.getByRole('dialog', { name: 'Call with Caller' })
    await call.locator('.call-line--you').first().waitFor({ timeout: 60000 })
    await call.locator('.call-line--agent').first().waitFor({ timeout: 60000 })
    await call.getByRole('button', { name: 'Hang up' }).click()
    await other.close()
    console.log('PASS a call in Firefox')
  }
} catch (error) {
  for (const [at, page] of pages.entries()) {
    await page.screenshot({ path: join(evidence, `failure-${at}.png`) })
    console.log(await page.locator('body').innerText())
  }
  throw error
} finally {
  await browser.close()
  proxy.close()
}
console.log('call-check browser passed')
process.exit(0)
