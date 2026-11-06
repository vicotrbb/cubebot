const tools = document.querySelector<HTMLElement>('[data-catalog-tools]')
const search = document.querySelector<HTMLInputElement>('[data-catalog-search]')
const count = document.querySelector<HTMLElement>('[data-catalog-count]')
const empty = document.querySelector<HTMLElement>('[data-catalog-empty]')
const filters = Array.from(
  document.querySelectorAll<HTMLButtonElement>('[data-filter]'),
)
const shelves = Array.from(
  document.querySelectorAll<HTMLElement>('[data-shelf]'),
)

let category = ''

function matches(item: HTMLElement, words: string[]): boolean {
  if (category && item.dataset.category !== category) return false
  const text = item.dataset.text ?? ''
  return words.every((word) => text.includes(word))
}

function apply(): void {
  const words = (search?.value ?? '').toLowerCase().split(/\s+/).filter(Boolean)
  let shown = 0
  for (const shelf of shelves) {
    const items = Array.from(
      shelf.querySelectorAll<HTMLElement>('[data-category]'),
    )
    let visible = 0
    for (const item of items) {
      item.hidden = !matches(item, words)
      if (!item.hidden) visible += 1
    }
    shelf.hidden = visible === 0
    shown += visible
  }
  if (count)
    count.textContent = shown === 1 ? '1 connector' : `${shown} connectors`
  if (empty) empty.hidden = shown > 0
}

function choose(button: HTMLButtonElement): void {
  category = button.dataset.filter ?? ''
  for (const filter of filters)
    filter.setAttribute('aria-pressed', String(filter === button))
  apply()
}

if (tools) tools.hidden = false
search?.addEventListener('input', apply)
for (const filter of filters)
  filter.addEventListener('click', () => choose(filter))
