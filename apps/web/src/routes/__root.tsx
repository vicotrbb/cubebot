import { QueryClientProvider } from '@tanstack/react-query'
import {
  createRootRoute,
  HeadContent,
  Outlet,
  Scripts,
} from '@tanstack/react-router'
import { queryClient } from '../app/queries'
import { THEME_BOOT } from '../app/theme'
import tokens from '../styles/tokens.css?url'
import base from '../styles/base.css?url'
import controls from '../styles/controls.css?url'

export const Route = createRootRoute({
  head: () => ({
    meta: [
      { charSet: 'utf-8' },
      {
        name: 'viewport',
        content: 'width=device-width, initial-scale=1, viewport-fit=cover',
      },
      { title: 'Cubebot' },
      { name: 'theme-color', content: '#f26a1b' },
      { name: 'apple-mobile-web-app-capable', content: 'yes' },
      { name: 'mobile-web-app-capable', content: 'yes' },
    ],
    links: [
      { rel: 'stylesheet', href: tokens },
      { rel: 'stylesheet', href: base },
      { rel: 'stylesheet', href: controls },
      { rel: 'manifest', href: '/manifest.webmanifest' },
      { rel: 'icon', href: '/favicon.svg', type: 'image/svg+xml' },
      { rel: 'apple-touch-icon', href: '/icons/icon-192.png' },
    ],
    scripts: [{ children: THEME_BOOT }],
  }),
  component: Root,
})

function Root() {
  return (
    <html lang="en" suppressHydrationWarning>
      <head>
        <HeadContent />
      </head>
      <body>
        <QueryClientProvider client={queryClient}>
          <Outlet />
        </QueryClientProvider>
        <Scripts />
      </body>
    </html>
  )
}
