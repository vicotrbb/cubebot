// Chat rendering, attachments, the file viewer, rooms, webhooks, usage, the Situation room and the
// activity badge in a real browser, on desktop and on a phone. Against a database-only stack with
// CUBEBOT_STORAGE_DIRECTORY and Vite on :3100, seeded by platform_browser_seed.py:
//   python3 tests/system/platform_browser_seed.py $S/seed.json
//   PLAYWRIGHT=/path/to/playwright/index.mjs S=<dir> node tests/system/platform_browser.mjs
// BROWSER=firefox with FIREFOX=<executable> runs it in Firefox.
import fs from 'node:fs'
import process from 'node:process'
import console from 'node:console'
const { chromium, firefox } = await import(process.env.PLAYWRIGHT)
const S = process.env.S
const seed = JSON.parse(fs.readFileSync(S + '/seed.json'))
const B = 'http://127.0.0.1:3100'
const kind = process.env.BROWSER === 'firefox' ? firefox : chromium
const opts = process.env.FIREFOX ? { executablePath: process.env.FIREFOX } : {}
const browser = await kind.launch(opts)
const fail = []
const ok = (c, w) => {
  console.log((c ? 'ok   ' : 'FAIL ') + w)
  if (!c) fail.push(w)
}
async function session(viewport, scheme, tag, extra = {}) {
  const ctx = await browser.newContext({
    viewport,
    colorScheme: scheme,
    ...extra,
  })
  const p = await ctx.newPage()
  const goto = p.goto.bind(p)
  p.goto = async (...a) => {
    await p.waitForTimeout(300)
    try {
      return await goto(...a)
    } catch (e) {
      if (!/ABORTED/.test(e.message)) throw e
      await p.waitForTimeout(600)
      return goto(...a)
    }
  }
  const errs = []
  p.on('pageerror', (e) => errs.push(e.message))
  p.on('console', (m) => {
    if (m.type() === 'error' && !/404|favicon|sandboxed/.test(m.text()))
      errs.push(m.text())
  })
  await p.goto(B + '/sign-in')
  await p.getByLabel(/login|username/i).fill('victor')
  await p.getByLabel(/password/i).fill('dev-password-123')
  await p.keyboard.press('Enter')
  await p.waitForURL((u) => !u.pathname.startsWith('/sign-in'), {
    timeout: 20000,
  })
  return {
    ctx,
    p,
    errs,
    shot: (n) =>
      p.screenshot({
        path: `${S}/shots/${process.env.BROWSER || 'chromium'}-${tag}-${n}.png`,
      }),
  }
}
fs.mkdirSync(S + '/shots', { recursive: true })
async function run(viewport, scheme, tag) {
  const { ctx, p, errs, shot } = await session(
    viewport,
    scheme,
    tag,
    viewport.width < 500
      ? { hasTouch: true, isMobile: process.env.BROWSER !== 'firefox' }
      : {},
  )
  const overflow = async (w) => {
    const o = await p.evaluate(
      () =>
        globalThis.document.documentElement.scrollWidth - globalThis.innerWidth,
    )
    ok(o <= 1, `${tag} no sideways scroll: ${w} (${o})`)
  }
  // Conversation
  await p.goto(`${B}/agents/${seed.ayla}`)
  await p.getByText('Totals').waitFor({ timeout: 15000 })
  await p.waitForTimeout(1500)
  ok(
    (await p.locator('.md strong', { hasText: 'Apr 12, 2025' }).count()) === 1,
    `${tag} bold renders`,
  )
  ok(
    !(await p.locator('.msg--agent').first().innerText()).includes('**'),
    `${tag} no raw asterisks`,
  )
  ok((await p.locator('.md table').count()) >= 1, `${tag} table`)
  ok((await p.locator('.md__code pre').count()) >= 1, `${tag} code`)
  ok(
    (await p.locator('.md input[type=checkbox]').count()) === 2,
    `${tag} task list`,
  )
  ok((await p.locator('.md del').count()) === 1, `${tag} strikethrough`)
  await p
    .locator('.katex')
    .first()
    .waitFor({ timeout: 8000 })
    .catch(() => {})
  ok((await p.locator('.katex').count()) >= 1, `${tag} math`)
  await p
    .locator('.md__diagram svg')
    .first()
    .waitFor({ timeout: 12000 })
    .catch(() => {})
  ok((await p.locator('.md__diagram svg').count()) >= 1, `${tag} mermaid`)
  ok(
    (await p.locator('.md img[src*="evil"]').count()) === 0,
    `${tag} outside image never loads`,
  )
  ok(
    (await p.locator('.md script').count()) === 0 &&
      (await p.locator('.md p', { hasText: 'Energy is' }).innerText()).includes(
        '<script>',
      ),
    `${tag} raw HTML stays visible text`,
  )
  ok(
    !(await p.locator('#root, body').first().innerText()).includes('Via room'),
    `${tag} room turn not in agent chat`,
  )
  ok(
    !(await p.locator('body').innerText()).includes('In room Test room'),
    `${tag} room text not in agent chat`,
  )
  await p.locator('.attachment-image img').first().waitFor({ timeout: 8000 })
  ok(
    (await p.locator('.attachment-image img').count()) === 1,
    `${tag} image attachment inline`,
  )
  ok(
    (await p.locator('.attachment-media audio').count()) === 1,
    `${tag} audio attachment player`,
  )
  ok(
    (await p.getByText('voice.wav').count()) >= 1,
    `${tag} sent-file tag hidden`,
  )
  ok(
    (await p.locator('.attachment-file').count()) === 1,
    `${tag} other file chip`,
  )
  await p.locator('.transcript').evaluate((e) => (e.scrollTop = 0))
  await p.waitForTimeout(300)
  await shot('chat-top')
  await overflow('chat')
  await p.locator('.transcript').evaluate((e) => (e.scrollTop = e.scrollHeight))
  await p.waitForTimeout(300)
  await shot('chat-bottom')
  await p.locator('.attachment-image').first().click()
  await p.locator('dialog.viewer img').waitFor()
  await shot('attachment-open')
  await p.keyboard.press('Escape')
  await p.waitForTimeout(200)
  // Room
  await p.goto(`${B}/rooms/${seed.room}`)
  await p.getByText('coding playbook').waitFor({ timeout: 10000 })
  ok(
    (await p.locator('.md strong', { hasText: 'accomplished' }).count()) === 1,
    `${tag} room markdown`,
  )
  await p.locator('.attachment-image img').first().waitFor({ timeout: 8000 })
  ok(true, `${tag} room attachment`)
  await shot('room')
  // Files
  await p.goto(`${B}/files`)
  await p.getByText('notes.md').waitFor({ timeout: 10000 })
  await shot('files')
  for (const [name, sel] of [
    ['a_small_map_of_home.txt', '.viewer__text'],
    ['notes.md', '.viewer__page .md h1'],
    ['orders.csv', '.viewer__table td'],
    ['config.json', '.viewer__text'],
    ['chart.png', '.viewer__picture img'],
    ['voice.wav', '.viewer__listen audio'],
    ['report.pdf', '.viewer__pdf canvas'],
    ['page.html', 'iframe.viewer__frame'],
    ['archive.zip', '.viewer__none'],
  ]) {
    await p.getByRole('button', { name, exact: true }).click()
    await p
      .locator('dialog.viewer ' + sel)
      .first()
      .waitFor({ timeout: 15000 })
      .then(() => ok(true, `${tag} opens ${name}`))
      .catch(() => ok(false, `${tag} opens ${name}`))
    ok(p.url().includes('open='), `${tag} ${name} in URL`)
    if (name === 'page.html') {
      await p.waitForTimeout(300)
      const t = await p.title()
      ok(t !== 'PWNED', `${tag} html script did not reach parent`)
      const inside = await p
        .frameLocator('iframe.viewer__frame')
        .locator('h1')
        .innerText()
      ok(inside === 'Page', `${tag} html script did not run`)
    }
    if (
      [
        'report.pdf',
        'notes.md',
        'orders.csv',
        'chart.png',
        'archive.zip',
      ].includes(name)
    )
      await shot('view-' + name)
    await p.goBack()
    await p.waitForTimeout(250)
    ok(
      (await p.locator('dialog.viewer').count()) === 0,
      `${tag} back closes ${name}`,
    )
  }
  await overflow('files')
  // Routines
  await p.goto(`${B}/agents/${seed.ayla}/routines`)
  await p
    .locator('.panel .title', { hasText: 'Deploy alert' })
    .waitFor({ timeout: 10000 })
  await p.locator('.hook summary', { hasText: 'Deliveries' }).click()
  await p.waitForTimeout(500)
  ok(
    (await p.locator('.hook__deliveries li').count()) >= 2,
    `${tag} deliveries listed`,
  )
  await p.locator('.hook summary', { hasText: 'How to call it' }).click()
  await p.waitForTimeout(150)
  await shot('routines')
  await overflow('routines')
  // Usage
  await p.goto(`${B}/agents/${seed.ayla}/usage`)
  await p.getByText('Last 30 days').waitFor({ timeout: 10000 })
  await p.waitForTimeout(300)
  await shot('usage')
  await overflow('usage')
  // Situation
  await p.goto(`${B}/situation`)
  await p.locator('.brief h2').waitFor({ timeout: 10000 })
  await p.waitForTimeout(800)
  await shot('situation')
  await overflow('situation')
  if (viewport.width > 500) {
    await p.locator('.bars svg rect[fill=transparent]').nth(24).hover()
    await p.waitForTimeout(120)
    await p.locator('.situation__body').screenshot({
      path: `${S}/shots/${process.env.BROWSER || 'chromium'}-${tag}-situation-full.png`,
    })
  } else {
    await p.evaluate(() => {
      const b = globalThis.document.querySelector('.situation__body')
      b.scrollTop = b.scrollHeight
    })
    await p.waitForTimeout(200)
    await shot('situation-end')
  }
  // Activity
  await p.goto(`${B}/`)
  await p.waitForTimeout(600)
  await p.goto(`${B}/activity`)
  await p.waitForTimeout(1200)
  ok(
    (await p
      .locator('.row__badge:visible')
      .filter({ hasNot: p.locator('xx') })
      .count()) === 0,
    `${tag} activity badge clears`,
  )
  await shot('activity')
  ok(
    errs.length === 0,
    `${tag} no console errors: ${errs.slice(0, 3).join(' | ')}`,
  )
  await ctx.close()
}
await run({ width: 1440, height: 900 }, process.env.SCHEME || 'light', 'desk')
await run({ width: 390, height: 844 }, process.env.SCHEME2 || 'dark', 'phone')
await browser.close()
console.log(fail.length ? `FAILED ${fail.length}` : 'ALL OK')
if (fail.length) process.exitCode = 1
