import { readFileSync, writeFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'

const root = join(dirname(fileURLToPath(import.meta.url)), '..', '..', '..')
const styles = join(root, 'apps/web/src/styles')
const shell = join(root, 'apps/web/src/features/shell/shell.css')
export const kitPath = join(root, 'modules/kit/kit.css')

const extras = `button,
input,
select,
textarea {
  font: inherit;
  color: inherit;
}
img,
svg,
video,
canvas,
iframe {
  max-width: 100%;
}
body {
  overflow-wrap: break-word;
}
.segmented {
  display: inline-flex;
  gap: 2px;
  max-width: 100%;
  overflow-x: auto;
}
.segmented > button {
  appearance: none;
  display: inline-flex;
  align-items: center;
  border: 0;
  background: none;
  color: var(--ink-soft);
  font-size: var(--t-sm);
  font-weight: var(--w-label);
  white-space: nowrap;
  cursor: pointer;
}
.segmented > [aria-pressed='true'] {
  color: var(--ink);
}
@media (pointer: coarse) {
  .segmented > button {
    min-height: 44px;
  }
}
@media (max-width: 640px) {
  .page__head,
  .page__body {
    padding-inline: 16px;
  }
}
.kit-grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(min(100%, 220px), 1fr));
  gap: 12px;
}
.kit-card {
  display: grid;
  align-content: start;
  gap: 6px;
  min-width: 0;
  overflow-x: auto;
  padding: 14px 16px;
  border-radius: var(--r-lg);
  background: var(--raised);
  box-shadow:
    inset 0 0 0 1px var(--line),
    var(--hairline);
  color: var(--ink);
  font-size: var(--t-md);
}
.kit-card h2,
.kit-card h3 {
  font-size: var(--t-lg);
  font-weight: var(--w-strong);
}
.kit-card p {
  color: var(--ink-soft);
  font-size: var(--t-sm);
}
.kit-stat {
  display: grid;
  gap: 2px;
}
.kit-stat__label {
  color: var(--ink-faint);
  font-size: var(--t-xs);
  font-weight: var(--w-label);
}
.kit-stat__value {
  font-size: var(--t-2xl);
  font-weight: var(--w-head);
  font-variant-numeric: tabular-nums;
  line-height: 1.2;
}
.kit-stat__note {
  color: var(--ink-soft);
  font-size: var(--t-xs);
}
`

function page(): string {
  const css = readFileSync(shell, 'utf8')
  const start = css.indexOf('.page {')
  const end = css.indexOf('.palette {')
  if (start < 0 || end < start) {
    throw new Error('shell.css no longer has the .page block before .palette')
  }
  return css.slice(start, end)
}

export function buildKit(): string {
  const parts = ['tokens.css', 'base.css', 'controls.css'].map((name) =>
    readFileSync(join(styles, name), 'utf8'),
  )
  return `${[...parts, page(), extras].join('\n')}`
    .replace(/#root,\s*body > div \{[^}]*\}\n*/g, '')
    .replace(/^[ \t]*overscroll-behavior: none;\n/gm, '')
    .replace(/@font-face\s*\{[^}]*\}\n*/g, '')
    .replace(/^[ \t]*\/\*[\s\S]*?\*\/[ \t]*\n/gm, '')
    .replace(/\/\*[\s\S]*?\*\//g, '')
    .replace(/\n{3,}/g, '\n\n')
    .trim()
    .concat('\n')
}

if (import.meta.url === pathToFileURL(process.argv[1] ?? '').href) {
  writeFileSync(kitPath, buildKit())
}
