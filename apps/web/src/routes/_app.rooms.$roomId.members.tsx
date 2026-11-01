import { createFileRoute } from '@tanstack/react-router'

// The members panel renders in the room layout; this route only opens it.
export const Route = createFileRoute('/_app/rooms/$roomId/members')({
  component: () => null,
})
