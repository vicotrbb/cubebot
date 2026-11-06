/** A form's GiB or MiB entry as bytes; empty means the deployment's value. */
export function bytesFrom(
  value: FormDataEntryValue | null,
  unit: number,
): number | null {
  const text = typeof value === 'string' ? value.trim() : ''
  if (!text) return null
  const number = Number(text)
  return Number.isFinite(number) ? Math.round(number * unit) : null
}

/** Bytes as a form's GiB or MiB entry, to two decimals at most. */
export function unitsOf(bytes: number | null | undefined, unit: number) {
  return bytes == null ? '' : String(Math.round((bytes / unit) * 100) / 100)
}
