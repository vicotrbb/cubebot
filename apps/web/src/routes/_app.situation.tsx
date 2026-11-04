import { createFileRoute } from '@tanstack/react-router'
import { Situation } from '../features/situation/Situation'

export const Route = createFileRoute('/_app/situation')({
  component: Situation,
})
