// Closes header disclosure menus on outside click, Escape, or opening another.
const menus = Array.from(
  document.querySelectorAll<HTMLDetailsElement>('details[data-menu]'),
)

function closeAll(except?: HTMLDetailsElement): void {
  for (const menu of menus) {
    if (menu !== except) menu.open = false
  }
}

for (const menu of menus) {
  menu.addEventListener('toggle', () => {
    if (menu.open) closeAll(menu)
  })
}

document.addEventListener('click', (event) => {
  const target = event.target as Node
  if (!menus.some((menu) => menu.contains(target))) closeAll()
})

document.addEventListener('keydown', (event) => {
  if (event.key !== 'Escape') return
  const open = menus.find((menu) => menu.open)
  if (!open) return
  open.open = false
  open.querySelector('summary')?.focus()
})
