// Shared files (ADRs 0059 and 0060) in real Chromium against the built console behind
// Caddy: the administrator uploads with the picker and by dropping a file, makes a folder,
// drags a file into it, renames and downloads it, shares the folder with a person and
// deletes a file; that person then finds the folder under Shared with me, read only, and
// downloads from it.
import assert from 'node:assert/strict'
import console from 'node:console'
import { Buffer } from 'node:buffer'
import { readFile } from 'node:fs/promises'
import process from 'node:process'
import { createRequire } from 'node:module'
import { join } from 'node:path'

const require = createRequire(import.meta.url)
const { chromium } = require(
  process.env.CUBEBOT_PLAYWRIGHT_MODULE || 'playwright',
)
const evidence = process.env.CUBEBOT_EVIDENCE_DIRECTORY
const marker = process.env.CUBEBOT_FILES_MARKER
const text = `console ${marker}\n`
const folder = `Console ${marker}`

async function signIn(browser, login, password) {
  const context = await browser.newContext({ acceptDownloads: true })
  const page = await context.newPage()
  await page.setViewportSize({ width: 1400, height: 1000 })
  await page.goto(process.env.CUBEBOT_CONSOLE)
  await page.getByLabel('Login', { exact: true }).fill(login)
  await page.getByLabel('Password', { exact: true }).fill(password)
  await page.getByRole('button', { name: 'Sign in', exact: true }).click()
  await page.getByRole('button', { name: 'Files', exact: true }).click()
  await page.getByRole('heading', { name: 'Files' }).waitFor()
  return page
}

const views = (page) => page.getByRole('navigation', { name: 'File views' })
const row = (page, name) =>
  page.locator('li.memory').filter({ hasText: name }).first()

async function download(page, name) {
  const [file] = await Promise.all([
    page.waitForEvent('download'),
    row(page, name).getByRole('link', { name: 'Download' }).click(),
  ])
  assert.equal(file.suggestedFilename(), name)
  return readFile(await file.path(), 'utf8')
}

const browser = await chromium.launch()
try {
  const page = await signIn(
    browser,
    process.env.CUBEBOT_CONSOLE_LOGIN,
    process.env.CUBEBOT_CONSOLE_PASSWORD,
  )
  await page.locator('input[type=file]').setInputFiles({
    name: `picked-${marker}.txt`,
    mimeType: 'text/plain',
    buffer: Buffer.from(text),
  })
  await row(page, `picked-${marker}.txt`).waitFor()

  // A drop of a file from the desktop, as the browser delivers it.
  const transfer = await page.evaluateHandle((name) => {
    const data = new globalThis.DataTransfer()
    data.items.add(new globalThis.File(['dropped bytes'], name))
    return data
  }, `dropped-${marker}.bin`)
  const zone = page.locator('section.file-drop')
  await zone.dispatchEvent('dragover', { dataTransfer: transfer })
  await zone.dispatchEvent('drop', { dataTransfer: transfer })
  await row(page, `dropped-${marker}.bin`).waitFor()

  await page.getByPlaceholder('Folder name').fill(folder)
  await page.getByRole('button', { name: 'Create folder', exact: true }).click()
  await row(page, `${folder}/`).waitFor()

  for (const name of [`picked-${marker}.txt`, `dropped-${marker}.bin`]) {
    await row(page, name).dragTo(row(page, `${folder}/`))
    await row(page, name).waitFor({ state: 'detached' })
  }

  await page.getByRole('button', { name: `${folder}/`, exact: true }).click()
  await row(page, `picked-${marker}.txt`).waitFor()
  await row(page, `picked-${marker}.txt`)
    .getByRole('button', { name: 'Rename', exact: true })
    .click()
  await page.getByLabel('New name').fill(`renamed-${marker}.txt`)
  await page.getByLabel('New name').press('Enter')
  await row(page, `renamed-${marker}.txt`).waitFor()
  assert.equal(await download(page, `renamed-${marker}.txt`), text)
  await page.screenshot({ path: join(evidence, 'files-folder.png') })

  await views(page).getByRole('button', { name: 'My files' }).click()
  await row(page, `${folder}/`)
    .getByRole('button', { name: 'Share', exact: true })
    .click()
  const dialog = page.getByRole('dialog', { name: `Share ${folder}` })
  await dialog
    .getByRole('button', { name: 'Add person, team or agent' })
    .click()
  await dialog.getByLabel('Shared with').selectOption('user')
  await dialog.getByLabel('Login').fill('nobody-here')
  await dialog.getByRole('button', { name: 'Save shares' }).click()
  await dialog.getByRole('alert').waitFor()
  await dialog.getByLabel('Login').fill(process.env.CUBEBOT_MEMBER_LOGIN)
  await dialog.getByLabel('Permission').selectOption('read')
  await page.screenshot({ path: join(evidence, 'files-share.png') })
  await dialog.getByRole('button', { name: 'Save shares' }).click()
  await dialog.waitFor({ state: 'hidden' })

  await page.locator('input[type=file]').setInputFiles({
    name: `trash-${marker}.txt`,
    mimeType: 'text/plain',
    buffer: Buffer.from('trash'),
  })
  page.once('dialog', (confirm) => confirm.accept())
  await row(page, `trash-${marker}.txt`)
    .getByRole('button', { name: 'Delete', exact: true })
    .click()
  await row(page, `trash-${marker}.txt`).waitFor({ state: 'detached' })
  assert.equal(await page.getByRole('alert').count(), 0)
  await page.screenshot({ path: join(evidence, 'files-mine.png') })

  const member = await signIn(
    browser,
    process.env.CUBEBOT_MEMBER_LOGIN,
    process.env.CUBEBOT_MEMBER_PASSWORD,
  )
  await views(member).getByRole('button', { name: 'Shared with me' }).click()
  await member.getByRole('button', { name: `${folder}/`, exact: true }).click()
  await row(member, `renamed-${marker}.txt`).waitFor()
  await row(member, 'read only').waitFor()
  assert.equal(
    await member.getByRole('button', { name: 'Upload files' }).count(),
    0,
  )
  assert.equal(
    await row(member, `renamed-${marker}.txt`)
      .getByRole('button', { name: 'Delete' })
      .count(),
    0,
  )
  assert.equal(await download(member, `renamed-${marker}.txt`), text)
  await member.screenshot({ path: join(evidence, 'files-shared-with-me.png') })
  console.log('console files ok')
} finally {
  await browser.close()
}
