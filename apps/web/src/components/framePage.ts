/** Blocks the network, so a page cannot call out with what it holds or load what it was not given. */
const POLICY =
  "default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; img-src data: blob:; font-src data:; media-src data: blob:"

/** Tells the console how tall the page is, so an inline preview fits it. */
const REPORT_HEIGHT = `<script>addEventListener('DOMContentLoaded',function(){var r=document.documentElement,s=function(){parent.postMessage({frameHeight:r.scrollHeight},'*')};new ResizeObserver(s).observe(r);s()})</script>`

/**
 * A page for a sandboxed frame: the policy comes first, where nothing in the page can undo it, and
 * the doctype stays ahead of everything so the page is not in quirks mode.
 * shortcut: the frame can still navigate itself, which no policy prevents; what it could carry off
 * is only what the page already holds, its own source.
 */
export function framed(source: string) {
  const page = source.replace(/^\s*<!doctype[^>]*>/i, '')
  return `<!doctype html><meta charset="utf-8"><meta http-equiv="Content-Security-Policy" content="${POLICY}"><style>:root{color-scheme:light;background:#fff}</style>${REPORT_HEIGHT}${page}`
}

/** A drawing centered on the same page. */
export function framedSvg(source: string) {
  return framed(
    `<style>body{margin:0;padding:12px;display:grid;place-items:center}svg{display:block;width:min(100%,560px);height:auto;max-height:92vh}</style>${source}`,
  )
}
