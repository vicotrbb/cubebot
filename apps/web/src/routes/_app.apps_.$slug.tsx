import { createFileRoute } from '@tanstack/react-router'
import { AppView } from '../features/apps/AppView'

export const Route = createFileRoute('/_app/apps_/$slug')({
  component: AppRoute,
})

function AppRoute() {
  const { slug } = Route.useParams()
  return <AppView key={slug} slug={slug} />
}
