import { useEffect, useState, type ReactNode } from 'react'
import type { Session } from '../../api/types'
import { ConfirmHost } from '../../components/primitives'
import { AgentHoverCard } from '../agent/AgentHoverCard'
import { FloatingPreviews } from '../desktop/FloatingPreview'
import { CommandPalette } from './CommandPalette'
import { MobileBar } from './MobileBar'
import { Sidebar } from './Sidebar'
import { useIsPhone } from './viewport'
import './shell.css'

export function Shell({
  session,
  children,
}: {
  session: Session
  children: ReactNode
}) {
  const phone = useIsPhone()
  const [palette, setPalette] = useState(false)
  useEffect(() => {
    const key = (event: KeyboardEvent) => {
      if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === 'k') {
        event.preventDefault()
        setPalette((open) => !open)
      }
    }
    window.addEventListener('keydown', key)
    return () => window.removeEventListener('keydown', key)
  }, [])
  return (
    <div className="shell">
      <a className="skip-link" href="#main">
        Skip to content
      </a>
      {!phone && <Sidebar onSearch={() => setPalette(true)} />}
      <main id="main" className="shell__main" tabIndex={-1}>
        {children}
      </main>
      <FloatingPreviews session={session} />
      <AgentHoverCard />
      {phone && <MobileBar />}
      <CommandPalette open={palette} onClose={() => setPalette(false)} />
      <ConfirmHost />
    </div>
  )
}
