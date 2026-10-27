// Blocks land once, when their group scrolls into view.

function reveal(
  entries: IntersectionObserverEntry[],
  observer: IntersectionObserver,
): void {
  for (const entry of entries) {
    if (!entry.isIntersecting) continue
    entry.target.classList.add('is-in')
    observer.unobserve(entry.target)
  }
}

const groups = document.querySelectorAll<HTMLElement>('[data-reveal]')
if ('IntersectionObserver' in window) {
  const observer = new IntersectionObserver(reveal, {
    rootMargin: '0px 0px -12% 0px',
  })
  groups.forEach((group) => observer.observe(group))
} else {
  groups.forEach((group) => group.classList.add('is-in'))
}
