import { createFileRoute } from '@tanstack/react-router'

export const Route = createFileRoute('/')({ component: Home })

function Home() {
  return (
    <main className="workspace">
      <header className="masthead">
        <a className="wordmark" href="/" aria-label="Cubebot home">
          <svg viewBox="0 0 32 32" fill="none" aria-hidden="true">
            <path d="m16 3 12 7v13l-12 7-12-7V10L16 3Z" />
            <path d="m4 10 12 7 12-7M16 17v13M10 6.5l12 7v6" />
          </svg>
          cubebot
        </a>
        <span className="badge">Project foundation</span>
      </header>
      <section className="introduction" aria-labelledby="welcome">
        <p className="eyebrow">A place for work that keeps going</p>
        <h1 id="welcome">A home for your agents.</h1>
        <p className="description">
          Persistent agents. Shared rooms. Knowledge that grows with the work.
          Built to live on your infrastructure.
        </p>
      </section>
      <section className="foundation" aria-labelledby="foundation">
        <span className="stage">01 / Foundation</span>
        <h2 id="foundation">The workspace starts here.</h2>
        <p>
          Cubebot is under construction. Agent creation, conversations, and
          rooms will arrive as the engine takes shape.
        </p>
        <div className="future" aria-label="Planned workspace areas">
          <span>Agents</span>
          <span>Rooms</span>
          <span>Knowledge</span>
        </div>
      </section>
      <footer>Your infrastructure. Your agents. Your work.</footer>
    </main>
  )
}
