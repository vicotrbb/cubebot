import type { PDFPageProxy } from 'pdfjs-dist'
import { useEffect, useRef, useState } from 'react'
import { Alert, Skeleton } from '../../components/primitives'

/** Pages longer than this stop rendering; the download has the rest. */
const PAGES = 200

/** PDFs drawn here rather than by the browser: Android's and an iOS home-screen app's built-in
 *  viewers show nothing, or only the first page, inside a page. Loaded with the first PDF. */
export function PdfPages({ blob }: { blob: Blob }) {
  const host = useRef<HTMLDivElement>(null)
  const [state, setState] = useState<'loading' | 'done' | 'failed'>('loading')
  const [total, setTotal] = useState(0)
  useEffect(() => {
    let current = true
    const element = host.current
    if (!element) return
    async function draw() {
      const [pdfjs, worker] = await Promise.all([
        import('pdfjs-dist'),
        import('pdfjs-dist/build/pdf.worker.min.mjs?url'),
      ])
      pdfjs.GlobalWorkerOptions.workerSrc = worker.default
      const document = await pdfjs.getDocument({
        data: new Uint8Array(await blob.arrayBuffer()),
      }).promise
      if (current) setTotal(document.numPages)
      const shown = Math.min(document.numPages, PAGES)
      for (let number = 1; number <= shown && current; number++) {
        const page = await document.getPage(number)
        element?.append(
          await drawn(page, (element.parentElement?.clientWidth ?? 960) - 24),
        )
        if (number === 1 && current) setState('done')
      }
    }
    draw().catch(() => {
      if (current) setState('failed')
    })
    return () => {
      current = false
      element?.replaceChildren()
    }
  }, [blob])
  return (
    <>
      {state === 'failed' && (
        <Alert>This PDF could not be shown. Download it to open it.</Alert>
      )}
      {state === 'loading' && (
        <div className="viewer__wait" role="status" aria-label="Loading PDF">
          <Skeleton lines={6} />
        </div>
      )}
      <div ref={host} className="viewer__pdf" />
      {total > PAGES && (
        <p className="viewer__cut">
          Showing {PAGES} of {total} pages. Download the file for all of it.
        </p>
      )}
    </>
  )
}

async function drawn(page: PDFPageProxy, available: number) {
  const natural = page.getViewport({ scale: 1 })
  const fit = Math.min(available, 960) / natural.width
  // Sharp on a phone's dense screen without an enormous canvas.
  const density = Math.min(window.devicePixelRatio || 1, 2)
  const viewport = page.getViewport({ scale: fit * density })
  const canvas = document.createElement('canvas')
  canvas.width = viewport.width
  canvas.height = viewport.height
  canvas.style.width = `${viewport.width / density}px`
  canvas.style.height = `${viewport.height / density}px`
  await page.render({ canvas, viewport }).promise
  return canvas
}
