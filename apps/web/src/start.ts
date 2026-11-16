import { createMiddleware, createStart } from '@tanstack/react-start'

export function frameAncestors(configured: string | undefined) {
  const sources = (configured ?? '').trim().split(/\s+/).filter(Boolean)
  const valid = sources.every((source) =>
    /^(https?:\/\/)?(\*\.)?[a-z0-9.-]+(:\d{1,5})?$/i.test(source),
  )
  return valid && sources.length > 0 ? sources.join(' ') : "'none'"
}

function hostKey(value: string) {
  return value
    .replace(/^[a-z][a-z0-9+.-]*:\/\//i, '')
    .replace(/\/.*$/s, '')
    .toLowerCase()
    .replace(/:(443|80)$/, '')
    .replace(/\.+$/, '')
}

export function meetingsHost(meetings?: string, console?: string) {
  const host = meetings ? hostKey(meetings) : undefined
  return host && host !== (console && hostKey(console)) ? host : undefined
}

export function onMeetingsHost(request: Request, host: string | undefined) {
  const hosts = [request.headers.get('host'), new URL(request.url).host]
  return !!host && hosts.some((value) => !!value && hostKey(value) === host)
}

const MEETING_PATHS =
  /^\/(m\/.+|(assets|fonts|icons)\/.+|favicon\.svg|manifest\.webmanifest|call-capture\.js|apple-touch-icon\.png)$/

export function meetingsAnswer(request: Request, host: string) {
  const url = new URL(request.url)
  const proto = request.headers
    .get('x-forwarded-proto')
    ?.split(',', 1)[0]
    ?.trim()
  if (proto === 'http')
    return new Response(null, {
      status: 308,
      headers: { location: `https://${host}${url.pathname}${url.search}` },
    })
  if (!MEETING_PATHS.test(url.pathname))
    return new Response('Not found', { status: 404 })
}

const MEETINGS_HOST = meetingsHost(
  process.env.CUBEBOT_MEETINGS_ORIGIN,
  process.env.CUBEBOT_PUBLIC_ORIGIN,
)

const HEADERS = {
  'content-security-policy': `frame-ancestors ${frameAncestors(process.env.CUBEBOT_FRAME_ANCESTORS)}; form-action 'self'; base-uri 'self'; object-src 'none'`,
  'x-content-type-options': 'nosniff',
  'referrer-policy': 'same-origin',
}

const secured = createMiddleware().server(async ({ next, request }) => {
  const meeting = onMeetingsHost(request, MEETINGS_HOST)
  const answer =
    meeting && MEETINGS_HOST && meetingsAnswer(request, MEETINGS_HOST)
  if (answer) return answer
  const result = await next()
  for (const [name, value] of Object.entries(HEADERS))
    result.response.headers.set(name, value)
  if (meeting)
    result.response.headers.set('strict-transport-security', 'max-age=31536000')
  return result
})

export const startInstance = createStart(() => ({
  requestMiddleware: [secured],
}))
