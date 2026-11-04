import { createFileRoute } from '@tanstack/react-router'
import { useSession } from '../app/queries'
import { Empty, Skeleton } from '../components/primitives'
import { Broadcast } from '../features/broadcast/Broadcast'

export const Route = createFileRoute('/_app/broadcast')({
  component: BroadcastScreen,
})

function BroadcastScreen() {
  const session = useSession()
  if (!session.data)
    return (
      <div className="page__body">
        <Skeleton lines={2} width="40%" />
      </div>
    )
  if (!session.data.administrator)
    return (
      <Empty title="Broadcast is for administrators.">
        It reaches every agent on the installation at once.
      </Empty>
    )
  return <Broadcast session={session.data} />
}
