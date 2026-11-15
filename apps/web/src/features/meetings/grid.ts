export type Grid = { columns: number; width: number; height: number }

export function bestGrid(
  count: number,
  width: number,
  height: number,
  gap: number,
  ratio = 16 / 9,
): Grid {
  let best: Grid = { columns: 1, width: 0, height: 0 }
  for (let columns = 1; columns <= Math.max(1, count); columns += 1) {
    const rows = Math.ceil(count / columns)
    const across = (width - gap * (columns - 1)) / columns
    const down = (height - gap * (rows - 1)) / rows
    const tileWidth = Math.floor(Math.min(across, down * ratio))
    if (tileWidth > best.width)
      best = {
        columns,
        width: tileWidth,
        height: Math.floor(tileWidth / ratio),
      }
  }
  return best
}
