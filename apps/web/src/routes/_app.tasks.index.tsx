import { createFileRoute } from '@tanstack/react-router'
import { Tasks } from '../features/tasks/Tasks'

export const Route = createFileRoute('/_app/tasks/')({ component: Tasks })
