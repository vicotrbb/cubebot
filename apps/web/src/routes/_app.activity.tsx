import { createFileRoute } from '@tanstack/react-router'
import { Activity } from '../features/activity/Activity'

export const Route = createFileRoute('/_app/activity')({ component: Activity })
