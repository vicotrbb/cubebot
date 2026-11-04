import { createFileRoute } from '@tanstack/react-router'
import { PageView } from '../features/knowledge/PageView'

export const Route = createFileRoute('/_app/kb/$pageId')({ component: Open })

function Open() {
  const { pageId } = Route.useParams()
  return <PageView key={pageId} id={pageId} />
}
