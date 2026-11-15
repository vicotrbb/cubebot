import { createMiddleware, createStart } from '@tanstack/react-start'

export function frameAncestors(configured: string | undefined) {
  const sources = (configured ?? '').trim().split(/\s+/).filter(Boolean)
  const valid = sources.every((source) =>
    /^(https?:\/\/)?(\*\.)?[a-z0-9.-]+(:\d{1,5})?$/i.test(source),
  )
  return valid && sources.length > 0 ? sources.join(' ') : "'none'"
}

const HEADERS = {
  'content-security-policy': `frame-ancestors ${frameAncestors(process.env.CUBEBOT_FRAME_ANCESTORS)}; form-action 'self'; base-uri 'self'; object-src 'none'`,
  'x-content-type-options': 'nosniff',
  'referrer-policy': 'same-origin',
}

const secured = createMiddleware().server(async ({ next }) => {
  const result = await next()
  for (const [name, value] of Object.entries(HEADERS))
    result.response.headers.set(name, value)
  return result
})

export const startInstance = createStart(() => ({
  requestMiddleware: [secured],
}))
