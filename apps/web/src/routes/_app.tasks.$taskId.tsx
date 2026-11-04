import { createFileRoute } from '@tanstack/react-router'
import { useSession } from '../app/queries'
import { TaskView } from '../features/tasks/TaskView'

export const Route = createFileRoute('/_app/tasks/$taskId')({
  component: TaskScreen,
})

function TaskScreen() {
  const { taskId } = Route.useParams()
  const session = useSession()
  // Remounting per task drops the room's stream with the task it followed.
  return session.data ? (
    <TaskView key={taskId} taskId={taskId} session={session.data} />
  ) : null
}
