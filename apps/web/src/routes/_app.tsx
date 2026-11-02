import { createFileRoute, Navigate, Outlet } from '@tanstack/react-router'
import { useEffect } from 'react'
import { useInstallationStream } from '../app/events'
import { usePreferences, useSession } from '../app/queries'
import { applyTheme } from '../app/theme'
import { Shell } from '../features/shell/Shell'

// The shell is personal and lives behind a session cookie, so it renders in the browser; the
// server sends the pending component in its place.
export const Route = createFileRoute('/_app')({
  ssr: false,
  pendingComponent: Opening,
  component: App,
})

function Opening() {
  return (
    <main className="shell-loading" role="status">
      Opening Cubebot…
    </main>
  )
}

function App() {
  const session = useSession()
  const preferences = usePreferences()
  useInstallationStream(Boolean(session.data))
  useEffect(() => {
    if (preferences.data) applyTheme(preferences.data.theme)
  }, [preferences.data])
  if (session.isPending) return <Opening />
  if (session.isError)
    return (
      <main className="shell-loading" role="alert">
        Cubebot is unavailable. Retry shortly.
      </main>
    )
  if (!session.data) return <Navigate to="/sign-in" replace />
  return (
    <Shell session={session.data}>
      <Outlet />
    </Shell>
  )
}
