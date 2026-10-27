// The header underlines the section currently in the middle of the viewport.

const links = Array.from(
  document.querySelectorAll<HTMLAnchorElement>('.top__nav a[href^="#"]'),
)

function mark(id: string): void {
  for (const link of links) {
    link.classList.toggle('is-current', link.hash === `#${id}`)
  }
}

function watch(entries: IntersectionObserverEntry[]): void {
  for (const entry of entries) {
    if (entry.isIntersecting) mark(entry.target.id)
  }
}

if ('IntersectionObserver' in window) {
  const observer = new IntersectionObserver(watch, {
    rootMargin: '-35% 0px -55% 0px',
  })
  for (const link of links) {
    const target = document.querySelector(link.hash)
    if (target) observer.observe(target)
  }
}
