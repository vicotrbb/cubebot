import type { APIRoute } from 'astro'
import { connectors } from '../data/connectors'

const pages = [
  '/',
  '/integrations/',
  '/download/',
  '/docs/install/',
  '/docs/features/',
  '/docs/architecture/',
  '/docs/security/',
  '/changelog/',
  '/about/',
  ...connectors.map((c) => `/integrations/${c.slug}/`),
]

export const GET: APIRoute = ({ site }) => {
  const urls = pages
    .map((path) => `  <url><loc>${new URL(path, site).href}</loc></url>`)
    .join('\n')
  const body = `<?xml version="1.0" encoding="UTF-8"?>\n<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">\n${urls}\n</urlset>\n`
  return new Response(body, { headers: { 'content-type': 'application/xml' } })
}
