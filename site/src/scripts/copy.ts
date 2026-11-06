// Copy buttons next to commands; the command stays selectable without JS.
document
  .querySelectorAll<HTMLButtonElement>('[data-copy]')
  .forEach((button) => {
    button.hidden = false
    button.addEventListener('click', async () => {
      const text = button.dataset.copy ?? ''
      try {
        await navigator.clipboard.writeText(text)
        button.dataset.state = 'done'
        button.setAttribute('aria-label', 'Copied')
      } catch {
        button.dataset.state = 'failed'
        button.setAttribute(
          'aria-label',
          'Copy failed, select the text instead',
        )
      }
      setTimeout(() => {
        delete button.dataset.state
        button.setAttribute('aria-label', 'Copy command')
      }, 1800)
    })
  })
