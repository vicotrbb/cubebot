export type Line = {
  speaker: string
  kind: string
  at: string | null
  text: string
}

export type Sheet = {
  kind: 'meeting' | 'call'
  title: string
  started_at: string | null
  ended_at: string | null
  details: [string, string][]
  summary: string | null
  lines: Line[]
}

export type Format = 'markdown' | 'json' | 'pdf'

export function fileName(sheet: Sheet, extension: string) {
  const slug = sheet.title
    .normalize('NFKD')
    .replace(/[̀-ͯ]/g, '')
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-|-$/g, '')
  const day = sheet.started_at?.slice(0, 10)
  return `${[slug || sheet.kind, day].filter(Boolean).join('-')}.${extension}`
}

export function markdown(sheet: Sheet, time: (at: string) => string) {
  const parts = [`# ${sheet.title}`]
  if (sheet.details.length)
    parts.push(
      sheet.details
        .map(([label, value]) => `- **${label}:** ${value}`)
        .join('\n'),
    )
  if (sheet.summary)
    parts.push(
      `## Summary\n\n${sheet.summary
        .trim()
        .replace(/^#{1,6}(?= )/gm, (marks) =>
          '#'.repeat(Math.min(6, Math.max(3, marks.length + 1))),
        )}`,
    )
  parts.push(
    sheet.lines.length
      ? `## Transcript\n\n${sheet.lines
          .map(
            (line) =>
              `**${line.speaker}**${line.at ? ` · ${time(line.at)}` : ''}\n${line.text}`,
          )
          .join('\n\n')}`
      : '## Transcript\n\nNothing was said.',
  )
  return `${parts.join('\n\n')}\n`
}

export function json(sheet: Sheet) {
  const { details, lines, ...rest } = sheet
  return `${JSON.stringify(
    { ...rest, details: Object.fromEntries(details), transcript: lines },
    null,
    2,
  )}\n`
}

async function shared(file: File) {
  if (
    !matchMedia('(pointer: coarse)').matches ||
    !navigator.canShare?.({ files: [file] })
  )
    return false
  try {
    await navigator.share({ files: [file], title: file.name })
  } catch (error) {
    if (!(error instanceof DOMException && error.name === 'AbortError'))
      return false
  }
  return true
}

export async function save(name: string, type: string, text: string) {
  const file = new File([text], name, { type })
  if (await shared(file)) return
  const link = document.createElement('a')
  link.href = URL.createObjectURL(file)
  link.download = name
  link.click()
  setTimeout(() => URL.revokeObjectURL(link.href), 1000)
}
