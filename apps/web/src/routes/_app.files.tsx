import { createFileRoute } from '@tanstack/react-router'
import { Files } from '../features/files/Files'

export const Route = createFileRoute('/_app/files')({ component: FilesPage })

function FilesPage() {
  return (
    <div className="page">
      <header className="page__head">
        <h1>Files</h1>
        <span className="faint small">
          Shared files stay available while agents sleep.
        </span>
      </header>
      <div className="page__body page__body--list">
        <Files />
      </div>
    </div>
  )
}
