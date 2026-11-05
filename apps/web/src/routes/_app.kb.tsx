import { createFileRoute, Outlet, useParams } from '@tanstack/react-router'
import { KnowledgeTree } from '../features/knowledge/Knowledge'
import { useIsPhone } from '../features/shell/viewport'

export const Route = createFileRoute('/_app/kb')({ component: KnowledgeBase })

function KnowledgeBase() {
  const phone = useIsPhone()
  const params = useParams({ strict: false })
  const current = params.pageId
  return (
    <div className="kb">
      {(!phone || !current) && <KnowledgeTree current={current} />}
      {(!phone || current) && (
        <div className="kb-main">
          <Outlet />
        </div>
      )}
    </div>
  )
}
