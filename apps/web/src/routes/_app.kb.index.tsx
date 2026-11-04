import { createFileRoute } from '@tanstack/react-router'
import { Home } from '../features/knowledge/PageView'

export const Route = createFileRoute('/_app/kb/')({ component: Home })
