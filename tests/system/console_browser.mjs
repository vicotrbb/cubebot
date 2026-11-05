// The console's knowledge base, agent card, files page and providers page in a real browser,
// on desktop and on a phone. Against a stack with no agents' sandboxes needed: a disposable
// database, the orchestrator with CUBEBOT_STORAGE_DIRECTORY and a credential key, and Vite.
// Needs an account (CUBEBOT_TEST_LOGIN, CUBEBOT_TEST_PASSWORD), an agent named Ayla, and no
// provider configured, and calls the provider's listing through a mock.
//   PLAYWRIGHT=/path/to/playwright/index.mjs SHOTS=/tmp/shots node tests/system/console_browser.mjs
// BROWSER=firefox with FIREFOX=<executable> runs it in Firefox, the browser most at risk here.
import console from 'node:console'
import { Buffer } from 'node:buffer'
import process from 'node:process'

const { chromium, firefox } = await import(process.env.PLAYWRIGHT)
const B = process.env.CUBEBOT_TEST_ORIGIN ?? 'http://127.0.0.1:3100'
const LOGIN = process.env.CUBEBOT_TEST_LOGIN
const PASSWORD = process.env.CUBEBOT_TEST_PASSWORD
const SHOTS = process.env.SHOTS
const browserKind = process.env.BROWSER === 'firefox' ? firefox : chromium
const opts = process.env.FIREFOX ? { executablePath: process.env.FIREFOX } : {}
const browser = await browserKind.launch(opts)
const fail = []
const check = (ok, what) => {
  console.log((ok ? 'ok   ' : 'FAIL ') + what)
  if (!ok) fail.push(what)
}
async function session(viewport, extra = {}) {
  const ctx = await browser.newContext({
    viewport,
    colorScheme: 'dark',
    ...extra,
  })
  const page = await ctx.newPage()
  const goto = page.goto.bind(page)
  // Firefox aborts a navigation started while another settles; a person would not notice.
  page.goto = async (...a) => {
    await page.waitForTimeout(300)
    try {
      return await goto(...a)
    } catch (e) {
      if (!/ABORTED/.test(e.message)) throw e
      await page.waitForTimeout(500)
      return goto(...a)
    }
  }
  page.on('pageerror', (e) => {
    console.log('PAGEERROR', e.message.slice(0, 300))
    fail.push('pageerror ' + e.message.slice(0, 80))
  })
  page.on('console', (m) => {
    if (
      m.type() === 'error' &&
      !/40[0-9]|Failed to load resource/.test(m.text())
    )
      console.log('CONSOLE', m.text().slice(0, 300))
  })
  await page.goto(B + '/sign-in')
  await page.fill(
    'input[autocomplete="username"], input[name="login"], input >> nth=0',
    LOGIN,
  )
  await page.fill('input[type="password"]', PASSWORD)
  await page.keyboard.press('Enter')
  await page.waitForURL((u) => !u.pathname.startsWith('/sign-in'))
  return { ctx, page }
}
const shot = (page, name) =>
  SHOTS && page.screenshot({ path: `${SHOTS}/${name}.png` })
const { page } = await session({ width: 1440, height: 900 })

// hover card
await page.evaluate(() => globalThis.fetch('/api/kb/trash'))
const row = page.locator('.row--agent', { hasText: 'Ayla' }).first()
await row.waitFor()
await row.hover({ position: { x: 60, y: 10 } })
await page.waitForTimeout(500)
const card = page.locator('.agent-card')
check(await card.isVisible(), 'hover card opens over an agent row')
check(
  ((await card.innerText()).includes('Ayla') &&
    (await card.innerText()).includes('Asleep')) ||
    (await card.innerText()).includes('Ready'),
  'card names the agent and what it does',
)
const a = await card.boundingBox()
await page.mouse.move(120, (await row.boundingBox()).y + 12, { steps: 3 })
await page.waitForTimeout(80)
const b = await card.boundingBox()
check(a && b && a.x !== b.x, 'card follows the pointer')
await page.waitForTimeout(200)
await shot(page, 'hover')
await page.mouse.move(800, 500)
await page.waitForTimeout(100)
check(
  !(await card.isVisible().catch(() => false)),
  'card closes when the pointer leaves',
)

// panel tabs
await row.click()
await page.waitForURL(/agents\//)
await page.getByRole('link', { name: 'Details' }).click()
await page.waitForTimeout(600)
await shot(page, 'panel-tabs')
const kbSwitch = page.getByRole('switch', { name: /knowledge base/ })
check(
  (await kbSwitch.getAttribute('aria-checked')) === 'true',
  'knowledge base capability defaults to on',
)

// files
await page.goto(B + '/files')
await page.waitForTimeout(1500)
await shot(page, 'files')
const drop = await page.locator('.file-drop').boundingBox()
const main = await page.locator('.page__body').boundingBox()
check(
  Math.abs(drop.x - main.x - (main.x + main.width - drop.x - drop.width)) < 4,
  'files column is centered in the pane',
)

// providers
await page.route('**/api/provider/models', async (route) => {
  const body = JSON.parse(route.request().postData())
  if (body.api_key === 'bad')
    return route.fulfill({
      status: 422,
      body: '{"error":"provider_rejected"}',
      contentType: 'application/json',
    })
  const models = Array.from({ length: 454 }, (_, i) => ({
    id: i ? `vendor${i % 9}/model-${i}:free` : 'anthropic/claude-sonnet-5',
    context_window: 200000 + i,
    tools: i % 7 !== 3,
  }))
  return route.fulfill({
    status: 200,
    body: JSON.stringify(models),
    contentType: 'application/json',
  })
})
await page.goto(B + '/platform/providers')
await page.locator('.provider-card').first().waitFor()
await shot(page, 'providers-closed')
check(
  (await page.locator('.provider-card').count()) === 3,
  'three provider cards',
)
check(
  (await page.locator('.provider-card').first().innerText()).includes(
    'Not configured',
  ),
  'unconfigured status shows',
)
await page.getByRole('button', { name: /OpenRouter/ }).click()
await page.fill('input[type="password"]', 'bad')
await page.keyboard.press('Enter')
await page.waitForTimeout(300)
check(
  await page.getByText('OpenRouter rejected this key').isVisible(),
  'a rejected key says so',
)
await page.fill('input[type="password"]', 'sk-or-v1-good')
await page.keyboard.press('Enter')
await page.waitForTimeout(400)
await page.fill('input[type="search"][aria-label="Search models"]', 'claude')
await page.waitForTimeout(100)
await page
  .locator('.provider-model', { hasText: 'anthropic/claude-sonnet-5' })
  .click()
await page.waitForTimeout(250)
await shot(page, 'providers-open')
const win = await page.inputValue('input[type="number"]')
check(win === '200000', 'choosing a model fills its reported window: ' + win)
await page.getByRole('button', { name: 'Use OpenRouter' }).click()
await page.waitForTimeout(600)
check(
  (await page.locator('.provider-card').nth(2).innerText()).includes('In use'),
  'saved provider reads In use',
)
// switch to OpenAI, OpenRouter's key is kept
await page.getByRole('button', { name: /OpenAI/ }).click()
await page.fill('input[type="password"]', 'sk-openai')
await page.keyboard.press('Enter')
await page.waitForTimeout(400)
await page.locator('.provider-model').first().click()
await page.getByRole('button', { name: 'Switch agents to OpenAI' }).click()
await page.waitForTimeout(700)
check(
  (await page.locator('.provider-card').nth(2).innerText()).includes(
    'Key saved',
  ),
  'the provider stepping aside keeps its key',
)
check(
  (await page.locator('.provider-card').nth(0).innerText()).includes('In use'),
  'the new provider is in use',
)
await shot(page, 'providers-final')

// knowledge base
await page.goto(B + '/kb')
await page.waitForTimeout(1500)
await shot(page, 'kb-home')
await page.getByRole('button', { name: 'New page' }).click()
await page.waitForURL(/kb\/[0-9a-f-]{36}/)
const id = page.url().split('/').pop()
await page.locator('.kb-title').fill('Deploying Cubebot')
await page.locator('.kb-prose').click()
// Typing across a save: the cursor must stay where it is.
await page.keyboard.type('Before')
await page.waitForTimeout(1500)
await page.keyboard.type(' after the save.')
check(
  (await page.locator('.kb-prose').innerText()).includes(
    'Before after the save.',
  ),
  'a save never moves the cursor',
)
await page.keyboard.press('Enter')
await page.keyboard.type('Intro text with ')
await page.keyboard.press('ControlOrMeta+b')
await page.keyboard.type('bold')
await page.keyboard.press('ControlOrMeta+b')
await page.keyboard.press('Enter')
await page.keyboard.type('/head')
await page.waitForTimeout(200)
await shot(page, 'kb-slash')
check(
  await page.locator('.kb-slash').isVisible(),
  'slash menu opens and filters',
)
await page.keyboard.press('Enter')
await page.keyboard.type('Steps')
await page.keyboard.press('Enter')
await page.keyboard.type('/to-do')
await page.waitForTimeout(150)
await page.keyboard.press('Enter')
await page.keyboard.type('Install it')
await page.keyboard.press('Enter')
await page.keyboard.type('Run it')
await page.keyboard.press('Enter')
await page.keyboard.press('Enter')
await page.keyboard.type('/table')
await page.waitForTimeout(150)
await page.keyboard.press('Enter')
await page.keyboard.type('Name')
await page.keyboard.press('Tab')
await page.keyboard.type('Value')
await page.waitForTimeout(100)
await page.locator('.kb-prose > p').last().click()
await page.keyboard.type('/diagram')
await page.waitForTimeout(150)
await page.keyboard.press('Enter')
await page.waitForTimeout(1800)
check(
  (await page.locator('.kb-diagram__art svg').count()) === 1,
  'a diagram draws',
)
await shot(page, 'kb-page')
await page.waitForTimeout(1200)
check(
  (await page.locator('.kb-bar__state').innerText()) === 'Saved',
  'autosave settles on Saved',
)
const got = await page.evaluate(
  async (id) => (await globalThis.fetch(`/api/kb/pages/${id}`)).json(),
  id,
)
console.log('--- stored markdown ---\n' + got.body + '\n---')
check(/\*\*bold\*\*/.test(got.body), 'bold is Markdown')
check(/## Steps|# Steps/.test(got.body), 'heading is Markdown')
check(/- \[ \] Install it/.test(got.body), 'task list is Markdown')
check(/\| *Name *\| *Value *\|/.test(got.body), 'table is Markdown')
check(/```mermaid/.test(got.body), 'diagram is a mermaid fence')

// image upload
const png = Buffer.from(
  'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8DwHwAFBQIAX8jx0gAAAABJRU5ErkJggg==',
  'base64',
)
await page.locator('.kb-prose > p').last().click()
await page.keyboard.type('/image')
await page.waitForTimeout(150)
const chooser = page.waitForEvent('filechooser')
await page.keyboard.press('Enter')
await (
  await chooser
).setFiles({ name: 'dot.png', mimeType: 'image/png', buffer: png })
await page.waitForTimeout(800)
check(
  (await page.locator('.kb-prose img').count()) === 1,
  'an uploaded image shows in the page',
)
const loaded = await page
  .locator('.kb-prose img')
  .evaluate((img) => img.naturalWidth)
check(loaded === 1, 'the image bytes actually load')
check(
  (await page.locator('.kb-file').count()) === 1,
  'the upload is listed under attachments',
)
await page.waitForTimeout(1200)

// a block moves by its handle
const paragraph = page.locator('.kb-prose > p', { hasText: 'Before after' })
await paragraph.hover()
await page.waitForTimeout(150)
const grip = await page.locator('.kb-handle__grip').boundingBox()
const heading = await page
  .locator('.kb-prose > h1', { hasText: 'Steps' })
  .boundingBox()
await page.mouse.move(grip.x + 8, grip.y + 10)
await page.mouse.down()
await page.mouse.move(grip.x + 40, grip.y + 60, { steps: 4 })
await page.mouse.move(heading.x + 40, heading.y + heading.height + 2, {
  steps: 6,
})
await page.mouse.up()
await page.waitForTimeout(1200)
const order = await page.evaluate(() =>
  [...globalThis.document.querySelector('.kb-prose').children].map((el) =>
    el.textContent.slice(0, 12),
  ),
)
console.log('order', order)
check(
  order.indexOf('Before after') > order.indexOf('Steps'),
  'dragging the handle moves the block',
)
// reload keeps everything
await page.reload()
await page
  .locator('.kb-diagram__art svg')
  .waitFor({ timeout: 10000 })
  .catch(() => {})
check(
  (await page.locator('.kb-title').inputValue()) === 'Deploying Cubebot',
  'title survives a reload',
)
check(
  (await page.locator('.kb-prose ul[data-type="taskList"] > li').count()) === 2,
  'tasks survive a reload',
)
check(
  (await page.locator('.kb-prose table').count()) === 1,
  'table survives a reload',
)
check(
  (await page.locator('.kb-diagram__art svg').count()) === 1,
  'diagram survives a reload',
)
check(
  (await page.locator('.kb-prose img').count()) === 1,
  'image survives a reload',
)

// someone else's save arrives live, and a conflict is offered
const rev = (
  await page.evaluate(
    async (id) => (await globalThis.fetch(`/api/kb/pages/${id}`)).json(),
    id,
  )
).revision
await page.evaluate(
  async ([id, rev]) =>
    globalThis.fetch(`/api/kb/pages/${id}`, {
      method: 'PATCH',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({
        expected_revision: rev,
        body: '# Written elsewhere\n\nHello from another tab.',
      }),
    }),
  [id, rev],
)
await page.waitForTimeout(6000)
check(
  (await page.locator('.kb-prose').innerText()).includes(
    'Hello from another tab',
  ),
  "another author's save reaches an idle page",
)
const rev2 = (
  await page.evaluate(
    async (id) => (await globalThis.fetch(`/api/kb/pages/${id}`)).json(),
    id,
  )
).revision
await page.locator('.kb-prose').click()
await page.keyboard.press('ControlOrMeta+End')
await page.evaluate(
  async ([id, rev]) =>
    globalThis.fetch(`/api/kb/pages/${id}`, {
      method: 'PATCH',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({
        expected_revision: rev,
        body: 'Their newer text.',
      }),
    }),
  [id, rev2],
)
await page.keyboard.type(' my words')
await page.waitForTimeout(1300)
check(
  await page.getByText('Someone else changed this page').isVisible(),
  'a conflict stops and asks',
)
await shot(page, 'kb-conflict')
await page.getByRole('button', { name: 'Keep mine' }).click()
await page.waitForTimeout(700)
const after = await page.evaluate(
  async (id) => (await globalThis.fetch(`/api/kb/pages/${id}`)).json(),
  id,
)
check(after.body.includes('my words'), 'keeping mine saves mine over theirs')

// history
await page.getByRole('button', { name: 'Version history' }).click()
await page.waitForTimeout(500)
await shot(page, 'kb-history')
await page.keyboard.press('Escape')

// agent-written markdown is not rewritten by opening it
const wrote =
  '# From an agent\n\n* star bullet\n* second\n\nSome   spaced    text\n\n| a | b |\n|---|---|\n| 1 | 2 |\n'
const created = await page.evaluate(
  async (body) =>
    (
      await globalThis.fetch('/api/kb/pages', {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({
          request_id: globalThis.crypto.randomUUID(),
          parent: null,
          kind: 'page',
          title: 'Agent page',
          body,
        }),
      })
    ).json(),
  wrote,
)
await page.waitForTimeout(400)
await page
  .goto(`${B}/kb/${created.id}`)
  .catch(() => page.goto(`${B}/kb/${created.id}`))
await page.waitForTimeout(1800)
const stillThere = await page.evaluate(
  async (id) => (await globalThis.fetch(`/api/kb/pages/${id}`)).json(),
  created.id,
)
check(
  stillThere.body === wrote && stillThere.revision === 1,
  'opening a page never rewrites what an agent wrote',
)
check(
  (await page.locator('.kb-prose ul li').count()) === 2,
  'agent Markdown renders as blocks',
)
await shot(page, 'kb-agent-page')

// nested + folder + search + trash
await page.goto(B + '/kb')
await page.waitForTimeout(400)
await page.locator('.kb-tree__head summary').click()
await page
  .locator('.kb-tree__head .menu__item', { hasText: 'New folder' })
  .click()
await page.waitForURL(/kb\/[0-9a-f-]{36}/)
await page.waitForTimeout(300)
await page.locator('.kb-title').fill('Guides')
await page.waitForTimeout(1100)
await shot(page, 'kb-folder')
await page.locator('.kb-search input').fill('agent')
await page.waitForTimeout(700)
check((await page.locator('.kb-hit').count()) >= 1, 'search finds pages')
await shot(page, 'kb-search')
await page.locator('.kb-search input').fill('')
await page.locator('.kb-row', { hasText: 'Agent page' }).hover()
await page
  .locator('.kb-row', { hasText: 'Agent page' })
  .locator('summary')
  .click()
await page.getByRole('button', { name: 'Move to trash' }).first().click()
await page.waitForTimeout(500)
check(
  (await page.locator('.kb-row', { hasText: 'Agent page' }).count()) === 0,
  'trash removes it from the tree',
)
await page.getByRole('button', { name: 'Trash', exact: true }).click()
await page.waitForTimeout(400)
await shot(page, 'kb-trash')
await page.getByRole('button', { name: 'Restore' }).click()
await page.waitForTimeout(500)
await page.keyboard.press('Escape')
await page.waitForTimeout(200)
check(
  (await page.locator('.kb-row', { hasText: 'Agent page' }).count()) === 1,
  'restore brings it back',
)

// light theme
const light = await session(
  { width: 1440, height: 900 },
  { colorScheme: 'light' },
)
await light.page.goto(`${B}/kb/${id}`)
await light.page.waitForTimeout(1500)
await shot(light.page, 'kb-light')

// phone PWA
const phone = await session(
  { width: 390, height: 844 },
  { hasTouch: true, isMobile: true, deviceScaleFactor: 2 },
)
await phone.page.goto(B + '/')
await phone.page
  .locator('.row__doing')
  .first()
  .waitFor({ timeout: 8000 })
  .catch(() => {})
await shot(phone.page, 'phone-home')
check(
  await phone.page.locator('.row__doing').first().isVisible(),
  'phone rows say what the agent is doing',
)
await phone.page.goto(B + '/kb')
await phone.page.waitForTimeout(1500)
await shot(phone.page, 'phone-kb')
await phone.page.locator('.kb-row__link', { hasText: 'Deploying' }).tap()
await phone.page.waitForTimeout(1500)
await shot(phone.page, 'phone-kb-page')
check(
  await phone.page.locator('.kb-touchbar').isVisible(),
  'phone gets the touch toolbar',
)
check(
  (await phone.page.locator('.kb-tree').count()) === 0,
  'a phone page takes the whole screen',
)
const overflow = await phone.page.evaluate(
  () => globalThis.document.documentElement.scrollWidth - globalThis.innerWidth,
)
check(overflow <= 0, 'no sideways scroll on a phone page: ' + overflow)
await phone.page.goto(B + '/platform/providers')
await phone.page.waitForTimeout(1500)
await shot(phone.page, 'phone-providers')
await phone.page.getByRole('button', { name: /OpenRouter/ }).tap()
await phone.page.waitForTimeout(500)
await shot(phone.page, 'phone-providers-open')
await phone.page.goto(B + '/files')
await phone.page.waitForTimeout(1500)
await shot(phone.page, 'phone-files')
console.log(
  fail.length ? `\n${fail.length} FAILED:\n` + fail.join('\n') : '\nall passed',
)
await browser.close()
process.exit(fail.length ? 1 : 0)
