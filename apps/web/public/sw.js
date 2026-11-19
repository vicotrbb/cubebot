// Pages and the API always go to the network, so nothing authenticated is stored by accident.
const CACHE = 'cubebot-shell-v1'

self.addEventListener('install', () => self.skipWaiting())
self.addEventListener('activate', (event) => {
  event.waitUntil(
    caches
      .keys()
      .then((keys) =>
        Promise.all(
          keys.filter((key) => key !== CACHE).map((key) => caches.delete(key)),
        ),
      )
      .then(() => self.clients.claim()),
  )
})

self.addEventListener('fetch', (event) => {
  const url = new URL(event.request.url)
  const asset =
    event.request.method === 'GET' &&
    url.origin === location.origin &&
    (url.pathname.startsWith('/assets/') ||
      url.pathname.startsWith('/fonts/') ||
      url.pathname.startsWith('/icons/'))
  if (!asset) return
  event.respondWith(
    caches.open(CACHE).then(async (cache) => {
      const hit = await cache.match(event.request)
      if (hit) return hit
      const response = await fetch(event.request)
      if (response.ok) cache.put(event.request, response.clone())
      return response
    }),
  )
})

self.addEventListener('push', (event) => {
  const message = event.data?.json() ?? {}
  event.waitUntil(
    self.registration.showNotification(message.title || 'Cubebot', {
      body: message.body || '',
      tag: message.tag,
      icon: '/icons/icon-192.png',
      badge: '/icons/badge-96.png',
      data: { url: message.url || '/activity' },
    }),
  )
})

self.addEventListener('notificationclick', (event) => {
  event.notification.close()
  const url = new URL(event.notification.data?.url || '/', location.origin)
  event.waitUntil(
    self.clients
      .matchAll({ type: 'window', includeUncontrolled: true })
      .then((windows) => {
        const open = windows.find(
          (client) => new URL(client.url).origin === location.origin,
        )
        if (!open) return self.clients.openWindow(url.href)
        return open
          .focus()
          .then((client) => client.navigate?.(url.href))
          .catch(() => undefined)
      }),
  )
})
