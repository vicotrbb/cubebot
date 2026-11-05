import { createFileRoute } from '@tanstack/react-router'

export const Route = createFileRoute('/_app/rooms/$roomId/members')({
  component: () => null,
})
