// Tasks, code rendering, coding work, previews, routines and Teams and access in a real browser, on
// desktop and on a phone, against the stack tasks_browser_seed.py seeded and Vite on :3100:
//   python3 tests/system/tasks_browser_seed.py $S/seed.json
//   PLAYWRIGHT=/path/to/playwright/index.mjs S=<dir> node tests/system/tasks_browser.mjs
// BROWSER=firefox with FIREFOX=<executable> runs it in Firefox. Screenshots land in $S.
import fs from 'node:fs'
import process from 'node:process'
import console from 'node:console'
import { execFileSync, spawn } from 'node:child_process'
import { clearInterval, setInterval } from 'node:timers'
const { chromium, firefox } = await import(process.env.PLAYWRIGHT)
const S = process.env.S
const seed = JSON.parse(fs.readFileSync(`${S}/seed.json`))
const B = 'http://127.0.0.1:3100'
const kind = process.env.BROWSER === 'firefox' ? firefox : chromium
const browser = await kind.launch(
  process.env.FIREFOX ? { executablePath: process.env.FIREFOX } : {},
)
const fail = []
const ok = (condition, what) => {
  console.log(`${condition ? 'ok   ' : 'FAIL '}${what}`)
  if (!condition) fail.push(what)
}

async function open(viewport, colorScheme, extra = {}) {
  const context = await browser.newContext({
    viewport,
    colorScheme,
    serviceWorkers: 'block',
    ...extra,
  })
  const page = await context.newPage()
  // Firefox aborts a navigation that lands during the sign-in redirect; the second one holds.
  const goto = page.goto.bind(page)
  page.goto = async (...args) => {
    await page.waitForTimeout(300)
    try {
      return await goto(...args)
    } catch (error) {
      if (!/ABORTED/.test(error.message)) throw error
      await page.waitForTimeout(600)
      return goto(...args)
    }
  }
  page.errors = []
  page.on('pageerror', (error) => {
    // serviceWorkers: 'block' reads the property in every frame, and a sandboxed one refuses.
    // No sandbox answers the desktop socket on a database-only stack.
    if (!/serviceWorker|\/desktop\b/.test(error.message))
      page.errors.push(error.message)
  })
  page.on('console', (message) => {
    if (
      message.type() === 'error' &&
      !/404|favicon|sandboxed|Content Security|WebSocket|ERR_ABORTED|\/desktop\b/.test(
        message.text(),
      )
    )
      page.errors.push(message.text())
  })
  await page.goto(`${B}/sign-in`)
  await page.getByLabel(/login|username/i).fill('victor')
  await page.getByLabel(/password/i).fill('dev-password-123')
  await page.keyboard.press('Enter')
  await page.waitForURL((url) => !url.pathname.startsWith('/sign-in'), {
    timeout: 30000,
  })
  return page
}
const noOverflow = (page) =>
  page.evaluate(() => {
    const root = globalThis.document.documentElement
    return root.scrollWidth <= root.clientWidth + 1
  })
const shot = (page, name) => page.screenshot({ path: `${S}/${name}.png` })

async function files(page) {
  await page.goto(`${B}/files`)
  const openFile = async (name) => {
    await page
      .getByRole('button', { name: new RegExp(`^(Open )?${name}`) })
      .first()
      .click()
    await page.getByRole('dialog', { name }).waitFor()
    return page.getByRole('dialog', { name })
  }
  const close = () =>
    page.getByRole('dialog').getByRole('button', { name: 'Close' }).click()
  let viewer = await openFile('subtitles.ts')
  await viewer.locator('.hljs-keyword').first().waitFor({ timeout: 20000 })
  ok(
    await viewer.locator('.codeblock__gutter').isVisible(),
    'code shows line numbers',
  )
  const scroll = await viewer.locator('.viewer__body').evaluate((body) => {
    const pre = body.querySelector('pre')
    return {
      height: pre.getBoundingClientRect().height,
      client: body.clientHeight,
    }
  })
  ok(
    scroll.height > scroll.client,
    'all of a long file is reachable, not cut at 24rem',
  )
  await shot(page, 'file-code')
  await close()
  viewer = await openFile('page.html')
  const frame = viewer.frameLocator('iframe')
  await frame.getByText('SCRIPT RAN').waitFor()
  ok((await page.title()) !== 'PWNED', 'a file page cannot reach the console')
  await viewer.getByRole('button', { name: 'Code', exact: true }).click()
  await viewer.locator('.hljs-tag').first().waitFor()
  ok(true, 'an HTML file has a preview and its code')
  await close()
  viewer = await openFile('flow.mmd')
  await viewer.locator('.kb-diagram__art svg').waitFor({ timeout: 20000 })
  ok(true, 'a Mermaid file draws')
  await close()
  viewer = await openFile('logo.svg')
  await viewer.frameLocator('iframe').locator('svg').waitFor()
  await close()
  viewer = await openFile('mystery.weird')
  await viewer.getByText('plain words').waitFor()
  ok(true, 'a file with an unknown name still opens when it reads as text')
  await close()
  viewer = await openFile('archive.bin')
  await viewer.getByText('cannot show').waitFor()
  ok(true, 'bytes that are not text say so')
  await close()
}

async function conversation(page, tag) {
  await page.goto(`${B}/agents/${seed.ids.Ayla}`)
  const banner = page.locator('.preview').first()
  await banner.waitFor({ timeout: 20000 })
  const frame = banner.frameLocator('iframe')
  await frame.getByText('Script ran inside the frame').waitFor()
  ok(
    (await page.title()) !== 'PWNED',
    'a message preview cannot reach the console',
  )
  await banner.getByRole('button', { name: 'Code', exact: true }).click()
  await banner.locator('.hljs-tag').first().waitFor()
  ok(true, 'a message preview switches to its code')
  await banner.getByRole('button', { name: 'Preview', exact: true }).click()
  await banner.getByRole('button', { name: /Expand/ }).click()
  await page.getByRole('dialog', { name: 'HTML' }).waitFor()
  await page.getByRole('dialog').getByRole('button', { name: 'Close' }).click()
  await page.locator('.kb-diagram__art svg').first().waitFor({ timeout: 20000 })
  ok(true, 'a Mermaid fence draws in a message')
  ok(
    (await page.locator('.preview').count()) === 4,
    'html, mermaid, svg and csv are previews',
  )
  await page.locator('.md .hljs-keyword').first().waitFor()
  ok(true, 'an ordinary fence is highlighted')
  await shot(page, `chat-previews-${tag}`)
  const log = page.locator('.worklog').first()
  await log.locator('.worklog__summary').click()
  const lines = await log.locator('.work__line').allTextContents()
  ok(
    lines.some((line) =>
      line.includes('Wrote subtitles.ts in movie-subtitle-finder'),
    ),
    'a write reads as a sentence',
  )
  ok(
    lines.some((line) => line.startsWith('Opened opensubtitles.stoplight.io')),
    'a page opens as a sentence',
  )
  ok(
    !lines.some((line) => line.includes('"session"')),
    'no raw JSON in the log',
  )
  await log
    .locator('.work', { hasText: 'Wrote subtitles.ts' })
    .locator('.work__summary')
    .click()
  await log
    .locator('.work', { hasText: 'Wrote subtitles.ts' })
    .locator('.hljs-keyword')
    .first()
    .waitFor()
  await log
    .locator('.work', { hasText: 'Edited subtitles.ts' })
    .locator('.work__summary')
    .click()
  ok(
    (await log.locator('.diff__line[data-sign="+"]').count()) === 2,
    'an edit reads as a diff',
  )
  await log
    .locator('.work', { hasText: 'Ran' })
    .locator('.work__summary')
    .click()
  await log.locator('.term__exit').waitFor()
  ok(true, 'a failed command shows its exit status in a terminal')
  await log.scrollIntoViewIfNeeded()
  await shot(page, `chat-work-${tag}`)
  ok(await noOverflow(page), `conversation fits (${tag})`)
}

async function tasks(page, tag, phone) {
  await page.goto(`${B}/tasks`)
  await page.getByRole('heading', { name: 'What do you want done?' }).waitFor()
  await page.locator('.task-row').first().waitFor()
  ok(
    (await page
      .locator('.tasks__pile[data-pile="needs_you"] .task-row')
      .count()) === 1,
    'a question sits under Needs you',
  )
  ok(
    await page
      .locator('.tasks__pile[data-pile="working"] .task-row')
      .filter({ hasText: 'Ayla: Compare' })
      .isVisible(),
    'a working task says who is on what',
  )
  await shot(page, `tasks-${tag}`)
  ok(await noOverflow(page), `tasks page fits (${tag})`)
  await page.goto(`${B}/tasks/${seed.working}`)
  await page
    .getByRole('heading', { name: 'Compare subtitle services' })
    .waitFor()
  const message = page.locator('.msg', { hasText: 'Ayla researches' })
  ok(
    (await message.locator('.msg__author').textContent()) === 'Orchestrator',
    'the orchestrator speaks in the room',
  )
  await page.getByText('still checking its limits').waitFor({ timeout: 15000 })
  ok(true, 'an agent’s reply shows as it is written')
  await page.locator('.worklog__now').first().waitFor()
  ok(
    (await page.locator('.worklog__now').first().textContent()).includes(
      'Wrote services.md',
    ),
    'the room shows what the agent is doing now',
  )
  if (phone) await page.locator('.task__plan > summary').click()
  await page.locator('.plan__item[data-status="doing"]').waitFor()
  ok(
    (await page.locator('.plan__item').count()) === 4,
    'the plan lists each agent’s items',
  )
  await shot(page, `task-${tag}`)
  ok(await noOverflow(page), `task page fits (${tag})`)
  if (!phone) {
    await page.goto(`${B}/rooms/new`)
    ok(
      !(await page
        .locator('.sidebar .row--room', { hasText: 'Compare subtitle' })
        .count()),
      'a task’s room is not in the room list',
    )
    const tree = page.locator('#task-tree')
    await tree.waitFor()
    ok(
      (await tree.locator('li').count()) === 2,
      'the sidebar tree lists running tasks',
    )
    await page.getByRole('button', { name: /Hide 2 running tasks/ }).click()
    await tree.waitFor({ state: 'detached' })
    await page.getByRole('button', { name: /Show 2 running tasks/ }).click()
    await tree.waitFor()
    await page
      .locator('.sidebar')
      .screenshot({ path: `${S}/sidebar-${tag}.png` })
  }
}

async function platform(page, tag) {
  await page.goto(`${B}/platform/teams`)
  await page.getByRole('heading', { name: 'Test Team' }).waitFor()
  const row = page.locator('.access__row', { hasText: 'Ayla' })
  await row.locator('select').selectOption('chat')
  await page.getByText('Can chat on Ayla.').waitFor()
  await page.waitForTimeout(400)
  ok(
    (await row.locator('select').inputValue()) === 'chat',
    'a level sticks once chosen',
  )
  await page
    .getByRole('combobox', { name: /Add someone/ })
    .selectOption({ label: 'maria' })
  await page.getByText('maria joined Test Team.').waitFor()
  await page.locator('.access__pick', { hasText: 'maria' }).click()
  await page
    .locator('.access__row', { hasText: 'Ayla' })
    .getByText('Can chat through Test Team')
    .waitFor()
  ok(true, 'a person sees what their team gives them')
  await shot(page, `teams-person-${tag}`)
  await page.locator('.access__pick', { hasText: 'Test Team' }).click()
  await shot(page, `teams-${tag}`)
  ok(await noOverflow(page), `teams fits (${tag})`)
  await page.goto(`${B}/platform/tasks`)
  await page.getByText('What it thinks with').waitFor()
  await page.getByRole('button', { name: /More: Agents per round/ }).click()
  await page.getByRole('button', { name: 'Save' }).click()
  await page.getByText(/Saved\. The orchestrator/).waitFor()
  ok(true, 'task settings save')
  await shot(page, `task-settings-${tag}`)
  ok(await noOverflow(page), `task settings fit (${tag})`)
  await page.goto(`${B}/platform/providers`)
  await page.getByText('Google Gemini').waitFor()
  ok(
    await page.getByText('xAI Grok').isVisible(),
    'Gemini and Grok are offered',
  )
  await page.getByRole('button', { name: /xAI Grok/ }).click()
  await shot(page, `providers-${tag}`)
}

async function routines(page, tag) {
  await page.goto(`${B}/agents/${seed.ids.Ayla}/routines`)
  // The running turn's browser work brings up a live window, which floats over this panel.
  const live = page.locator('.pip').getByRole('button', { name: 'Dismiss' })
  await live.click({ timeout: 3000 }).catch(() => {})
  await page.getByRole('button', { name: 'New routine' }).click()
  ok(
    !(await page.getByLabel(/^Cron/).count()),
    'a schedule is not asked for up front',
  )
  await page.getByRole('textbox', { name: 'Name', exact: true }).fill('Hooked')
  await page
    .getByRole('textbox', { name: 'Prompt', exact: true })
    .fill('Do the thing.')
  await page.getByRole('checkbox', { name: /From a webhook/ }).check()
  await shot(page, `routine-form-${tag}`)
  await page.getByRole('button', { name: 'Save' }).click()
  await page.getByText('Copy this secret now').waitFor()
  ok(
    await page.getByText('Runs from its webhook or by hand').isVisible(),
    'a routine runs from a webhook with no schedule',
  )
  await page.getByRole('button', { name: 'New routine' }).click()
  await page.getByRole('checkbox', { name: /On a schedule/ }).check()
  await page.getByRole('button', { name: 'Every hour' }).click()
  ok(
    (await page.getByLabel(/^Cron/).inputValue()) === '0 * * * *',
    'a preset fills the schedule',
  )
  await page.getByRole('button', { name: 'Cancel' }).click()
}

/** Puts two more agents on their desktops, or takes them off; a desktop counts while a running turn
 *  used the browser in the last two minutes. */
function desktops(status) {
  stacked = status === 'running'
  execFileSync('docker', [
    'exec',
    'cubebot-dev-pg',
    'psql',
    '-U',
    'cubebot',
    '-d',
    'app',
    '-qc',
    desktopSql(status),
  ])
}

function desktopSql(status) {
  const turns = seed.desktops.map((turn) => `'${turn}'`).join(',')
  return `UPDATE chat_turns SET status='${status}' WHERE id IN (${turns}); UPDATE turn_events SET created_at=clock_timestamp() WHERE turn_id IN (${turns},'${seed.running}')`
}

/** Desktop previews stack with one live, and the stack opens to choose. */
async function stack(page, tag, phone) {
  await desktops('running')
  await page.goto(`${B}/tasks`)
  if (phone) {
    await page
      .getByRole('button', { name: /more desktops/ })
      .waitFor({ timeout: 15000 })
    ok(
      (await page.locator('.pip-pill').count()) === 1,
      'on a phone several desktops are one line',
    )
    await page.getByRole('button', { name: /more desktops/ }).click()
    await page.locator('.pip__choice').first().waitFor()
    await shot(page, `pip-${tag}`)
    await page.locator('.pip__choice', { hasText: 'Kind guy' }).click()
    await page.locator('.pip-pill', { hasText: 'Kind guy is using' }).waitFor()
    ok(true, 'picking on a phone switches the line')
  } else {
    await page.locator('.pip__behind').nth(1).waitFor({ timeout: 15000 })
    ok(
      (await page.locator('.pip').count()) === 1,
      'several desktops stack as one window',
    )
    ok(
      (await page.locator('.pip__behind').count()) === 2,
      'the others peek out behind it',
    )
    await page.getByRole('button', { name: /Choose among 3 desktops/ }).click()
    await shot(page, `pip-${tag}`)
    await page.locator('.pip__choice', { hasText: 'Kind guy' }).click()
    await page.locator('.pip__name', { hasText: 'Kind guy' }).waitFor()
    ok(true, 'clicking picks which desktop to watch')
  }
  await desktops('cancelled')
}

let stacked = false
// A database-only stack has no agent to own a running turn, and resets it; keep it running.
const keepRunning = setInterval(() => {
  spawn('docker', [
    'exec',
    'cubebot-dev-pg',
    'psql',
    '-U',
    'cubebot',
    '-d',
    'app',
    '-qc',
    `UPDATE chat_turns SET status='running', error=NULL WHERE id='${seed.running}' AND status<>'running'; ${stacked ? desktopSql('running') : ''}`,
  ])
}, 500)
const desk = await open({ width: 1440, height: 900 }, 'light')
await files(desk)
await conversation(desk, 'desk')
await tasks(desk, 'desk', false)
await platform(desk, 'desk')
await routines(desk, 'desk')
await stack(desk, 'desk', false)
ok(
  desk.errors.length === 0,
  `no page errors on a desk ${desk.errors.join(' | ')}`,
)

const dark = await open({ width: 1440, height: 900 }, 'dark')
await conversation(dark, 'dark')
await tasks(dark, 'dark', false)
await dark.goto(`${B}/platform/teams`)
await dark.getByRole('heading', { name: 'Test Team' }).waitFor()
await shot(dark, 'teams-dark')
ok(
  dark.errors.length === 0,
  `no page errors in the dark ${dark.errors.join(' | ')}`,
)

const phone = await open({ width: 390, height: 844 }, 'light', {
  hasTouch: true,
  isMobile: true,
  deviceScaleFactor: 2,
})
await conversation(phone, 'phone')
await tasks(phone, 'phone', true)
await phone.goto(`${B}/`)
await phone.locator('#task-tree').waitFor()
ok(true, 'the phone’s Home carries the task tree')
await shot(phone, 'home-phone')
await phone.goto(`${B}/platform/teams`)
await phone.getByRole('heading', { name: 'Test Team' }).waitFor()
await shot(phone, 'teams-phone')
ok(await noOverflow(phone), 'teams fits a phone')
await stack(phone, 'phone', true)
ok(
  phone.errors.length === 0,
  `no page errors on a phone ${phone.errors.join(' | ')}`,
)
clearInterval(keepRunning)
await browser.close()
if (fail.length) {
  console.log(`${fail.length} FAILED`)
  process.exit(1)
}
console.log('all passed')
