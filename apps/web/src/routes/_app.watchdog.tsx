import { createFileRoute } from '@tanstack/react-router'
import { Watchdog } from '../features/watchdog/Watchdog'

export const Route = createFileRoute('/_app/watchdog')({ component: Watchdog })
