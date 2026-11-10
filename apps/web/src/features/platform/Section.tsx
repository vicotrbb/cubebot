import { useId, type ReactNode } from 'react'
import './platform.css'

export function Section({
  title,
  description,
  children,
}: {
  title: string
  description?: ReactNode
  children: ReactNode
}) {
  const id = useId()
  return (
    <section className="settings__section" aria-labelledby={id}>
      <header className="settings__intro">
        <h2 id={id}>{title}</h2>
        {description && <div className="settings__about">{description}</div>}
      </header>
      <div className="settings__body">{children}</div>
    </section>
  )
}

export function SaveBar({ children }: { children: ReactNode }) {
  return <div className="settings__save">{children}</div>
}
