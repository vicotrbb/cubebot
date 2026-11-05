/** Blocks the network, so a page cannot call out with what it holds or load what it was not given. */
const POLICY =
  "default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; img-src data: blob:; font-src data:; media-src data: blob:"

const REPORT_HEIGHT = `<script>addEventListener('DOMContentLoaded',function(){var r=document.documentElement,s=function(){parent.postMessage({frameHeight:r.scrollHeight},'*')};new ResizeObserver(s).observe(r);s()})</script>`

/**
 * The policy comes first, where nothing in the page can undo it; the doctype stays ahead of it, to avoid quirks mode.
 * shortcut: the frame can still navigate itself, which no policy prevents; it could carry off only its own source.
 */
export function framed(source: string) {
  const page = source.replace(/^\s*<!doctype[^>]*>/i, '')
  return `<!doctype html><meta charset="utf-8"><meta http-equiv="Content-Security-Policy" content="${POLICY}"><style>:root{color-scheme:light;background:#fff}</style>${REPORT_HEIGHT}${page}`
}

export function framedSvg(source: string) {
  return framed(
    `<style>body{margin:0;padding:12px;display:grid;place-items:center}svg{display:block;width:min(100%,560px);height:auto;max-height:92vh}</style>${source}`,
  )
}
