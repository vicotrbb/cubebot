import { readFileSync, writeFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'

const root = join(dirname(fileURLToPath(import.meta.url)), '..', '..', '..')
const styles = join(root, 'apps/web/src/styles')
const shell = join(root, 'apps/web/src/features/shell/shell.css')
export const kitPath = join(root, 'modules/kit/kit.css')

const extras = `.kit-grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(min(100%, 220px), 1fr));
  gap: 12px;
}
.kit-card {
  display: grid;
  align-content: start;
  gap: 6px;
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
  return css.slice(css.indexOf('.page {'), css.indexOf('.palette {'))
}

export function buildKit(): string {
  const parts = ['tokens.css', 'base.css', 'controls.css'].map((name) =>
    readFileSync(join(styles, name), 'utf8'),
  )
  return `${[...parts, page(), extras].join('\n')}`
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
