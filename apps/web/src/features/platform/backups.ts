import type { components } from '../../api/schema'

type Summary = components['schemas']['BackupSummary']

/** A backup id is its UTC start, such as 20261107T030000Z. */
export function backupDate(id: string): Date | null {
  const match = /^(\d{4})(\d{2})(\d{2})T(\d{2})(\d{2})(\d{2})Z$/.exec(id)
  if (!match) return null
  const iso = `${match[1]}-${match[2]}-${match[3]}T${match[4]}:${match[5]}:${match[6]}Z`
  return new Date(iso)
}

export function backupName(id: string) {
  const date = backupDate(id)
  if (!date) return id
  return date.toLocaleString(undefined, {
    weekday: 'short',
    month: 'short',
    day: 'numeric',
    hour: '2-digit',
    minute: '2-digit',
  })
}

export const STATUS: Record<string, { label: string; tone: string }> = {
  running: { label: 'Running', tone: 'paint' },
  complete: { label: 'Verified', tone: 'sage' },
  incomplete: { label: 'Verified, parts missing', tone: 'paint' },
  failed: { label: 'Failed', tone: 'danger' },
  deleted: { label: 'Removed by retention', tone: '' },
}

/** The newest backup that can be restored from, for the page's summary line. */
export function lastVerified(backups: Summary[]) {
  return backups.find(
    (backup) => backup.status === 'complete' || backup.status === 'incomplete',
  )
}

/** "3 min 20 s", from two Unix times. */
export function took(start: number, end: number | null | undefined) {
  if (end == null) return ''
  const seconds = Math.max(0, end - start)
  if (seconds < 60) return `${seconds} s`
  const minutes = Math.floor(seconds / 60)
  if (minutes < 60) return `${minutes} min ${seconds % 60} s`
  return `${Math.floor(minutes / 60)} h ${minutes % 60} min`
}
