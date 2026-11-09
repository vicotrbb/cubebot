import { QueryClientProvider } from '@tanstack/react-query'
import {
  createRootRoute,
  HeadContent,
  Outlet,
  Scripts,
} from '@tanstack/react-router'
import { queryClient } from '../app/queries'
import { RestoreGate } from '../features/restore/Restore'
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
      {
        name: 'description',
        content: 'The console of a self-hosted Cubebot installation.',
      },
      // Each installation is private; only the public site belongs in search results.
      { name: 'robots', content: 'noindex' },
      { name: 'theme-color', content: '#f26a1b' },
      { name: 'apple-mobile-web-app-capable', content: 'yes' },
      { name: 'mobile-web-app-capable', content: 'yes' },
      // iOS names the home-screen app after the page title ("Home · Cubebot") without this.
      { name: 'apple-mobile-web-app-title', content: 'Cubebot' },
    ],
    links: [
      { rel: 'stylesheet', href: tokens },
      { rel: 'stylesheet', href: base },
      { rel: 'stylesheet', href: controls },
      { rel: 'manifest', href: '/manifest.webmanifest' },
      { rel: 'icon', href: '/favicon.svg', type: 'image/svg+xml' },
      { rel: 'apple-touch-icon', href: '/apple-touch-icon.png' },
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
          <RestoreGate>
            <Outlet />
          </RestoreGate>
        </QueryClientProvider>
        <Scripts />
      </body>
    </html>
  )
}
