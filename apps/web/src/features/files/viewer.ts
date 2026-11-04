/** How a file shows in the console, decided from its name first: agents upload without a type. */
export type Kind =
  | 'image'
  | 'audio'
  | 'video'
  | 'pdf'
  | 'markdown'
  | 'csv'
  | 'json'
  | 'html'
  | 'text'
  | null

const IMAGES: Record<string, string> = {
  png: 'image/png',
  jpg: 'image/jpeg',
  jpeg: 'image/jpeg',
  gif: 'image/gif',
  webp: 'image/webp',
  avif: 'image/avif',
  bmp: 'image/bmp',
  ico: 'image/x-icon',
  svg: 'image/svg+xml',
}
const AUDIO: Record<string, string> = {
  mp3: 'audio/mpeg',
  m4a: 'audio/mp4',
  aac: 'audio/aac',
  wav: 'audio/wav',
  ogg: 'audio/ogg',
  oga: 'audio/ogg',
  opus: 'audio/ogg',
  flac: 'audio/flac',
  weba: 'audio/webm',
}
const VIDEO: Record<string, string> = {
  mp4: 'video/mp4',
  m4v: 'video/mp4',
  webm: 'video/webm',
  ogv: 'video/ogg',
  mov: 'video/quicktime',
}
const TEXT = new Set(
  `txt log ini cfg conf env toml yaml yml xml tsv sql sh bash zsh fish ps1 bat c h cc cpp hpp cs go rs
  java kt kts swift rb py pyi php pl lua r scala dart js jsx mjs cjs ts tsx css scss less vue svelte
  graphql proto dockerfile makefile justfile gitignore tex rst adoc diff patch`.split(
    /\s+/,
  ),
)
const NAMED: Record<string, Kind> = {
  md: 'markdown',
  markdown: 'markdown',
  csv: 'csv',
  json: 'json',
  jsonl: 'text',
  pdf: 'pdf',
  html: 'html',
  htm: 'html',
}

function extension(name: string) {
  const last = name.toLowerCase().split('/').pop() ?? ''
  return last.includes('.') ? (last.split('.').pop() ?? '') : last
}

function kindOfType(type: string | null | undefined): Kind {
  const base = type?.split(';')[0]?.trim().toLowerCase() ?? ''
  if (base === 'application/pdf') return 'pdf'
  if (base === 'text/html') return 'html'
  if (base === 'text/markdown') return 'markdown'
  if (base === 'text/csv') return 'csv'
  if (base === 'application/json') return 'json'
  const family = base.split('/')[0]
  if (family === 'image' || family === 'audio' || family === 'video')
    return family
  return family === 'text' ? 'text' : null
}

export function kindOf(name: string, type?: string | null): Kind {
  const ext = extension(name)
  if (ext in IMAGES) return 'image'
  if (ext in AUDIO) return 'audio'
  if (ext in VIDEO) return 'video'
  if (ext in NAMED) return NAMED[ext] ?? null
  if (TEXT.has(ext)) return 'text'
  return kindOfType(type)
}

/** The type a blob of this file takes. Only media types come through, so a blob URL can never
 *  be HTML that runs in the console's origin; HTML shows in a sandboxed frame instead. */
export function blobType(name: string, type?: string | null) {
  const ext = extension(name)
  const known = IMAGES[ext] ?? AUDIO[ext] ?? VIDEO[ext]
  if (known) return known
  const kind = kindOf(name, type)
  if (kind === 'pdf') return 'application/pdf'
  const media = kind === 'image' || kind === 'audio' || kind === 'video'
  return media ? (type?.split(';')[0]?.trim() ?? '') : 'text/plain'
}

/** Rows of comma-separated text, with quoted fields. */
export function parseTable(text: string): string[][] {
  const rows: string[][] = []
  let row: string[] = []
  for (const [, raw = '', end = ''] of text.matchAll(
    /("(?:[^"]|"")*"|[^,\r\n]*)(,|\r?\n|$)/g,
  )) {
    if (raw === '' && end === '' && row.length === 0) break
    const quoted = raw.startsWith('"')
    row.push(quoted ? raw.slice(1, -1).replaceAll('""', '"') : raw)
    if (end !== ',') {
      rows.push(row)
      row = []
    }
    if (end === '') break
  }
  return rows
}

/** Agents' sent files carry a tag so names never collide: `chart-1f3a9c2e.png` shows as `chart.png`. */
export function shownName(name: string) {
  return name.replace(/-[0-9a-f]{8}(\.[^.]+)?$/, '$1')
}

export function fileSize(bytes: number) {
  if (bytes < 1024) return `${bytes} B`
  const units = ['KiB', 'MiB', 'GiB', 'TiB']
  let value = bytes / 1024
  let unit = 0
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024
    unit++
  }
  return `${value.toFixed(1)} ${units[unit]}`
}
